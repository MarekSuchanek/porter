//! The same searches over the local fast path (parallel walk) and the generic
//! directory-by-directory walk (an OpenDAL backend, in-memory here).

use std::fs;
use std::sync::Arc;

use chrono::{TimeDelta, Utc};
use opendal::{Operator, services};
use porter_backends::OpendalProvider;
use porter_core::{EntryKind, Location, Target};
use porter_jobs::{CopyOptions, Job, SearchOptions, SizeFilter, copy, search};

/// photos/{a.jpg (2 MB), b.JPG, raw/c.jpg}, docs/{Report.pdf, notes.txt}, .cache/x.jpg
async fn trees() -> (tempfile::TempDir, [Target; 2]) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    fs::create_dir_all(root.join("photos/raw")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join(".cache")).unwrap();
    fs::write(root.join("photos/a.jpg"), vec![0u8; 2_000_000]).unwrap();
    fs::write(root.join("photos/b.JPG"), "b").unwrap();
    fs::write(root.join("photos/raw/c.jpg"), "c").unwrap();
    fs::write(root.join("docs/Report.pdf"), "pdf").unwrap();
    fs::write(root.join("docs/notes.txt"), "n").unwrap();
    fs::write(root.join(".cache/x.jpg"), "x").unwrap();
    let local = porter_backends::target(root.to_str().unwrap()).unwrap();

    let op = Operator::new(services::Memory::default()).unwrap();
    let location = Location {
        scheme: "memory".into(),
        authority: "t".into(),
        path: "/".into(),
    };
    let memory = Target::new(location, Arc::new(OpendalProvider::new("memory", "t", op)));
    let opts = CopyOptions {
        recursive: true,
        ..Default::default()
    };
    let report = copy(&Job::new(), std::slice::from_ref(&local), &memory, &opts)
        .await
        .unwrap();
    assert!(report.is_success(), "{report:?}");
    (dir, [local, memory.child("root")])
}

async fn names(root: &Target, opts: SearchOptions) -> Vec<String> {
    let mut found = Vec::new();
    let report = search(&Job::new(), root, &opts, |e| found.push(e.name))
        .await
        .unwrap();
    assert!(report.is_success(), "{report:?}");
    assert_eq!(report.files as usize, found.len());
    found.sort();
    found
}

#[tokio::test]
async fn filters_agree_on_both_walks() {
    let (_dir, roots) = trees().await;
    for root in &roots {
        let jpg = SearchOptions {
            name: Some("*.jpg".into()),
            ..Default::default()
        };
        assert_eq!(
            names(root, jpg.clone()).await,
            ["a.jpg", "b.JPG", "c.jpg"],
            "{root}"
        );

        let sensitive = SearchOptions {
            case_sensitive: true,
            ..jpg.clone()
        };
        assert_eq!(names(root, sensitive).await, ["a.jpg", "c.jpg"], "{root}");

        let hidden = SearchOptions {
            hidden: true,
            ..jpg.clone()
        };
        assert_eq!(
            names(root, hidden).await,
            ["a.jpg", "b.JPG", "c.jpg", "x.jpg"],
            "{root}"
        );

        let shallow = SearchOptions {
            max_depth: Some(2),
            ..jpg.clone()
        };
        assert_eq!(names(root, shallow).await, ["a.jpg", "b.JPG"], "{root}");

        let big = SearchOptions {
            sizes: vec![SizeFilter::parse("+1M").unwrap()],
            ..Default::default()
        };
        assert_eq!(names(root, big).await, ["a.jpg"], "{root}");

        let dirs = SearchOptions {
            kind: Some(EntryKind::Dir),
            ..Default::default()
        };
        assert_eq!(names(root, dirs).await, ["docs", "photos", "raw"], "{root}");

        let substring = SearchOptions {
            name: Some("report".into()),
            ..Default::default()
        };
        assert_eq!(names(root, substring).await, ["Report.pdf"], "{root}");

        let path = SearchOptions {
            regex: Some(r"^photos/raw/".into()),
            full_path: true,
            ..Default::default()
        };
        assert_eq!(names(root, path).await, ["c.jpg"], "{root}");

        let recent = SearchOptions {
            newer: Some(Utc::now() - TimeDelta::hours(1)),
            name: Some("*.txt".into()),
            ..Default::default()
        };
        // The in-memory backend records no modification times, and entries without
        // one never match a date filter; real remotes (SFTP, S3, FTP) do report them.
        let expected: &[&str] = if root.location.is_local() {
            &["notes.txt"]
        } else {
            &[]
        };
        assert_eq!(names(root, recent).await, expected, "{root}");
        let old = SearchOptions {
            older: Some(Utc::now() - TimeDelta::hours(1)),
            ..Default::default()
        };
        assert!(names(root, old).await.is_empty(), "{root}");
    }
}

#[tokio::test]
async fn limit_and_progress() {
    let (_dir, [local, _]) = trees().await;
    let job = Job::new();
    let opts = SearchOptions {
        limit: Some(2),
        ..Default::default()
    };
    let mut found = 0;
    let report = search(&job, &local, &opts, |_| found += 1).await.unwrap();
    assert_eq!((found, report.files), (2, 2));
    let progress = job.progress().snapshot();
    assert!(progress.scanned >= 2);
    assert!(!progress.scanning);

    let file = local.child("docs").child("notes.txt");
    assert!(
        search(&Job::new(), &file, &SearchOptions::default(), |_| {})
            .await
            .is_err()
    );
}
