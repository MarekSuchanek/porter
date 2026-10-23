//! Transfers between the local file system and an OpenDAL backend (in-memory here;
//! SFTP, S3 and FTP go through the same adapter).

use std::fs;
use std::path::Path;
use std::sync::Arc;

use futures::TryStreamExt;
use opendal::{Operator, services};
use porter_backends::OpendalProvider;
use porter_core::{Location, Target};
use porter_jobs::{CopyOptions, DeleteOptions, Job, copy, delete, move_to};

fn local(path: &Path) -> Target {
    porter_backends::target(path.to_str().unwrap()).unwrap()
}

fn memory() -> Target {
    let op = Operator::new(services::Memory::default()).unwrap();
    let location = Location {
        scheme: "memory".into(),
        authority: "test".into(),
        path: "/".into(),
    };
    Target::new(
        location,
        Arc::new(OpendalProvider::new("memory", "test", op)),
    )
}

fn recursive() -> CopyOptions {
    CopyOptions {
        recursive: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn round_trip_through_remote() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("photos");
    fs::create_dir_all(src.join("2024/summer")).unwrap();
    fs::write(src.join("index.txt"), "index").unwrap();
    fs::write(src.join("2024/summer/beach.jpg"), vec![7u8; 3 << 20]).unwrap();
    let remote = memory();

    let job = Job::new();
    let up = copy(&job, &[local(&src)], &remote, &recursive())
        .await
        .unwrap();
    assert!(up.is_success(), "{up:?}");
    assert_eq!((up.files, up.bytes), (2, 5 + (3 << 20)));
    let snapshot = job.progress().snapshot();
    assert_eq!(snapshot.bytes_done, snapshot.bytes_total);

    let listed: Vec<String> = remote
        .provider
        .list("/photos/2024/summer")
        .await
        .unwrap()
        .map_ok(|e| e.path)
        .try_collect()
        .await
        .unwrap();
    assert_eq!(listed, ["/photos/2024/summer/beach.jpg"]);

    let back = dir.path().join("restored");
    fs::create_dir(&back).unwrap();
    let down = copy(
        &Job::new(),
        &[remote.child("photos")],
        &local(&back),
        &recursive(),
    )
    .await
    .unwrap();
    assert!(down.is_success(), "{down:?}");
    assert_eq!(
        fs::read(back.join("photos/2024/summer/beach.jpg"))
            .unwrap()
            .len(),
        3 << 20
    );
    assert_eq!(
        fs::read_to_string(back.join("photos/index.txt")).unwrap(),
        "index"
    );
}

#[tokio::test]
async fn move_across_backends_copies_then_deletes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");
    fs::write(&file, "note").unwrap();
    let remote = memory();

    let report = move_to(
        &Job::new(),
        &[local(&file)],
        &remote,
        &CopyOptions::default(),
    )
    .await
    .unwrap();
    assert!(report.is_success(), "{report:?}");
    assert!(!file.exists());
    assert_eq!(
        remote.provider.stat("/note.txt").await.unwrap().size,
        Some(4)
    );

    let report = delete(
        &Job::new(),
        &[remote.child("note.txt")],
        &DeleteOptions::default(),
    )
    .await;
    assert!(report.is_success(), "{report:?}");
    assert!(remote.provider.stat("/note.txt").await.is_err());
}
