use std::fs;
use std::path::Path;

use porter_core::{Error, ModeSpec, Target};
use porter_jobs::{Conflict, CopyOptions, DeleteOptions, Job, chmod, copy, delete, mkdir, move_to};

fn target(path: &Path) -> Target {
    porter_backends::target(path.to_str().unwrap()).unwrap()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

/// `src/a.txt`, `src/nested/b.txt` and an empty `dst/`.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("nested")).unwrap();
    fs::write(src.join("a.txt"), "a").unwrap();
    fs::write(src.join("nested/b.txt"), "b").unwrap();
    fs::create_dir(dir.path().join("dst")).unwrap();
    dir
}

fn recursive(conflict: Conflict) -> CopyOptions {
    CopyOptions {
        recursive: true,
        conflict,
        ..Default::default()
    }
}

#[tokio::test]
async fn copies_file_into_directory_and_to_new_name() {
    let dir = fixture();
    let (a, dst) = (dir.path().join("src/a.txt"), dir.path().join("dst"));
    let job = Job::new();

    let report = copy(&job, &[target(&a)], &target(&dst), &CopyOptions::default())
        .await
        .unwrap();
    assert!(report.is_success(), "{report:?}");
    assert_eq!((report.files, report.bytes), (1, 1));
    assert_eq!(read(&dst.join("a.txt")), "a");

    let renamed = dst.join("renamed.txt");
    copy(
        &job,
        &[target(&a)],
        &target(&renamed),
        &CopyOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(read(&renamed), "a");
    let progress = job.progress().snapshot();
    assert_eq!((progress.files_done, progress.bytes_done), (2, 2));
}

#[tokio::test]
async fn multiple_sources_need_a_directory() {
    let dir = fixture();
    let src = dir.path().join("src");
    let result = copy(
        &Job::new(),
        &[target(&src.join("a.txt")), target(&src.join("nested"))],
        &target(&dir.path().join("missing")),
        &CopyOptions::default(),
    )
    .await;
    assert!(matches!(result, Err(Error::InvalidArgument(_))));
}

#[tokio::test]
async fn directories_need_recursive() {
    let dir = fixture();
    let report = copy(
        &Job::new(),
        &[target(&dir.path().join("src"))],
        &target(&dir.path().join("dst")),
        &CopyOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(report.errors.len(), 1);
    assert!(!dir.path().join("dst/src").exists());
}

#[tokio::test]
async fn copies_tree_preserving_metadata() {
    let dir = fixture();
    let src = dir.path().join("src");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::os::unix::fs::symlink("a.txt", src.join("link")).unwrap();
        fs::set_permissions(src.join("a.txt"), fs::Permissions::from_mode(0o640)).unwrap();
    }

    let report = copy(
        &Job::new(),
        &[target(&src)],
        &target(&dir.path().join("dst")),
        &recursive(Conflict::Fail),
    )
    .await
    .unwrap();
    assert!(report.is_success(), "{report:?}");
    assert_eq!(report.dirs, 2);

    let out = dir.path().join("dst/src");
    assert_eq!(read(&out.join("nested/b.txt")), "b");
    let (src_meta, dst_meta) = (
        fs::metadata(src.join("a.txt")).unwrap(),
        fs::metadata(out.join("a.txt")).unwrap(),
    );
    assert_eq!(src_meta.modified().unwrap(), dst_meta.modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(dst_meta.permissions().mode() & 0o777, 0o640);
        assert_eq!(fs::read_link(out.join("link")).unwrap(), Path::new("a.txt"));
    }
}

#[tokio::test]
async fn conflict_policies() {
    let dir = fixture();
    let a = target(&dir.path().join("src/a.txt"));
    let dst = dir.path().join("dst");
    fs::write(dst.join("a.txt"), "old").unwrap();
    let run = |conflict| {
        let (a, dst) = (a.clone(), target(&dst));
        async move {
            let opts = CopyOptions {
                conflict,
                ..Default::default()
            };
            copy(&Job::new(), &[a], &dst, &opts).await.unwrap()
        }
    };

    let report = run(Conflict::Fail).await;
    assert!(matches!(report.errors[..], [Error::AlreadyExists(_)]));
    assert_eq!(run(Conflict::Skip).await.skipped, 1);
    // The existing file was written after the source, so it is newer.
    assert_eq!(run(Conflict::Newer).await.skipped, 1);
    assert_eq!(read(&dst.join("a.txt")), "old");

    assert!(run(Conflict::Rename).await.is_success());
    assert!(run(Conflict::Rename).await.is_success());
    assert_eq!(read(&dst.join("a (1).txt")), "a");
    assert_eq!(read(&dst.join("a (2).txt")), "a");

    assert!(run(Conflict::Overwrite).await.is_success());
    assert_eq!(read(&dst.join("a.txt")), "a");
}

#[tokio::test]
async fn refuses_copying_onto_itself() {
    let dir = fixture();
    let src = target(&dir.path().join("src"));

    let into_self = copy(
        &Job::new(),
        std::slice::from_ref(&src),
        &src.child("nested"),
        &recursive(Conflict::Fail),
    )
    .await
    .unwrap();
    assert_eq!(into_self.errors.len(), 1);

    let a = src.child("a.txt");
    let same_file = copy(
        &Job::new(),
        std::slice::from_ref(&a),
        &a,
        &recursive(Conflict::Overwrite),
    )
    .await
    .unwrap();
    assert_eq!(same_file.errors.len(), 1);
    assert_eq!(read(&dir.path().join("src/a.txt")), "a");
}

#[tokio::test]
async fn cancelled_job_does_nothing() {
    let dir = fixture();
    let job = Job::new();
    job.cancel();
    let report = copy(
        &job,
        &[target(&dir.path().join("src"))],
        &target(&dir.path().join("dst")),
        &recursive(Conflict::Fail),
    )
    .await
    .unwrap();
    assert!(report.cancelled);
    assert!(!dir.path().join("dst/src").exists());
}

#[tokio::test]
async fn moves_by_rename_and_merges_into_existing_dir() {
    let dir = fixture();
    let (src, dst) = (dir.path().join("src"), dir.path().join("dst"));

    let report = move_to(
        &Job::new(),
        &[target(&src.join("a.txt"))],
        &target(&dst),
        &CopyOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(report.files, 1);
    assert!(!src.join("a.txt").exists());
    assert_eq!(read(&dst.join("a.txt")), "a");

    // `dst/nested` exists, so moving `src/nested` there merges via copy + delete.
    fs::create_dir(dst.join("nested")).unwrap();
    fs::write(dst.join("nested/keep.txt"), "keep").unwrap();
    let report = move_to(
        &Job::new(),
        &[target(&src.join("nested"))],
        &target(&dst),
        &CopyOptions::default(),
    )
    .await
    .unwrap();
    assert!(report.is_success(), "{report:?}");
    assert!(!src.join("nested").exists());
    assert_eq!(read(&dst.join("nested/b.txt")), "b");
    assert_eq!(read(&dst.join("nested/keep.txt")), "keep");
}

#[tokio::test]
async fn failed_merge_keeps_the_source() {
    let dir = fixture();
    let (src, dst) = (dir.path().join("src"), dir.path().join("dst"));
    fs::create_dir(dst.join("nested")).unwrap();
    fs::write(dst.join("nested/b.txt"), "other").unwrap();

    let report = move_to(
        &Job::new(),
        &[target(&src.join("nested"))],
        &target(&dst),
        &CopyOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(read(&src.join("nested/b.txt")), "b");
}

#[tokio::test]
async fn deletes_with_safety_checks() {
    let dir = fixture();
    let src = dir.path().join("src");
    let job = Job::new();

    let report = delete(&job, &[target(&src)], &DeleteOptions::default()).await;
    assert_eq!(report.errors.len(), 1, "directories need recursive");

    let opts = DeleteOptions {
        recursive: true,
        dry_run: true,
        ..Default::default()
    };
    let report = delete(&job, &[target(&src)], &opts).await;
    assert_eq!(report.planned.len(), 4);
    assert!(src.exists());

    let report = delete(&job, &[target(Path::new("/"))], &opts).await;
    assert!(report.planned.is_empty());
    assert_eq!(report.errors.len(), 1, "roots are refused");

    let opts = DeleteOptions {
        dry_run: false,
        ..opts
    };
    let report = delete(&job, &[target(&src)], &opts).await;
    assert!(report.is_success(), "{report:?}");
    assert_eq!((report.files, report.dirs), (2, 2));
    assert!(!src.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn chmod_recursive_symbolic() {
    use std::os::unix::fs::PermissionsExt;

    let dir = fixture();
    let src = dir.path().join("src");
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    fs::set_permissions(src.join("a.txt"), fs::Permissions::from_mode(0o600)).unwrap();

    let spec = ModeSpec::parse("go=rX").unwrap();
    let report = chmod(&Job::new(), &[target(&src)], &spec, true).await;
    assert!(report.is_success(), "{report:?}");
    assert_eq!(mode(&src.join("a.txt")), 0o644);
    assert_eq!(mode(&src.join("nested")), 0o755);
}

#[tokio::test]
async fn mkdir_with_parents() {
    let dir = fixture();
    let deep = target(&dir.path().join("x/y/z"));
    assert!(matches!(mkdir(&deep, false).await, Err(Error::NotFound(_))));
    mkdir(&deep, true).await.unwrap();
    mkdir(&deep, true).await.unwrap();
    assert!(dir.path().join("x/y/z").is_dir());
    assert!(matches!(
        mkdir(&deep, false).await,
        Err(Error::AlreadyExists(_))
    ));
}
