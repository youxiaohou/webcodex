use super::*;
fn root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
const ORIGIN: &str = "https://api.openai.com";

#[test]
fn live_owner_and_concurrent_recovery_cannot_archive() {
    let temp = root();
    let root = temp.path().canonicalize().unwrap();
    let owner = RunFence::acquire(&root, ORIGIN, "first").unwrap();
    let identity = observe(&root, ORIGIN, "first").unwrap().unwrap();
    assert_eq!(
        archive(&root, ORIGIN, "first", &identity.run_id, |_| true).unwrap_err(),
        "tunnel_owner_active"
    );
    assert!(owner.path().exists());
    owner.finish().unwrap();
    assert!(RunFence::acquire(&root, ORIGIN, "first").is_ok());
}
#[test]
fn archival_is_exact_retains_bytes_and_does_not_replay_on_repeat() {
    let temp = root();
    let root = temp.path().canonicalize().unwrap();
    let owner = RunFence::acquire(&root, ORIGIN, "first").unwrap();
    let identity = observe(&root, ORIGIN, "first").unwrap().unwrap();
    let path = owner.path().to_path_buf();
    let bytes = fs::read(&path).unwrap();
    drop(owner);
    assert_eq!(
        archive(&root, ORIGIN, "first", &identity.run_id, |_| false).unwrap_err(),
        "tunnel_owner_active"
    );
    assert!(archive(
        &root,
        ORIGIN,
        "wrong-profile-identity",
        &identity.run_id,
        |_| true
    )
    .is_err());
    assert!(archive(
        &root,
        ORIGIN,
        "first",
        &uuid::Uuid::new_v4().to_string(),
        |_| true
    )
    .is_err());
    let archived = archive(&root, ORIGIN, "first", &identity.run_id, |pid| {
        pid == identity.owner_pid
    })
    .unwrap();
    assert_eq!(fs::read(&archived).unwrap(), bytes);
    assert!(!path.exists());
    assert_eq!(
        archive(&root, ORIGIN, "first", &identity.run_id, |_| true).unwrap_err(),
        "tunnel_fence_missing"
    );
    let replacement = RunFence::acquire(&root, ORIGIN, "first").unwrap();
    drop(replacement);
    assert_eq!(
        archive(&root, ORIGIN, "first", &identity.run_id, |_| true).unwrap_err(),
        "tunnel_fence_changed"
    );
    assert_eq!(fs::read(&archived).unwrap(), bytes);
}
#[test]
fn legacy_and_invalid_markers_never_get_silently_upgraded_or_removed() {
    let temp = root();
    let root = temp.path().canonicalize().unwrap();
    let path = paths(&root, ORIGIN, "first").0;
    for (bytes, expected) in [
        (
            b"native-tunnel-run-v1\n".as_slice(),
            "tunnel_legacy_owner_unverifiable",
        ),
        (b"not valid".as_slice(), "tunnel_fence_invalid"),
    ] {
        let mut file = private_options()
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        assert_eq!(observe(&root, ORIGIN, "first").unwrap_err(), expected);
        assert_eq!(
            archive(
                &root,
                ORIGIN,
                "first",
                &uuid::Uuid::new_v4().to_string(),
                |_| true
            )
            .unwrap_err(),
            expected
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
#[test]
fn failed_archive_preserves_active_fence_and_existing_archive() {
    let temp = root();
    let root = temp.path().canonicalize().unwrap();
    let owner = RunFence::acquire(&root, ORIGIN, "first").unwrap();
    let identity = observe(&root, ORIGIN, "first").unwrap().unwrap();
    let path = owner.path().to_path_buf();
    drop(owner);
    let parent = root.join(format!(
        "{}.recovered",
        path.file_name().unwrap().to_string_lossy()
    ));
    private_directory(&parent).unwrap();
    let destination = parent.join(&identity.run_id);
    fs::write(&destination, b"existing history").unwrap();
    assert_eq!(
        archive(&root, ORIGIN, "first", &identity.run_id, |_| true).unwrap_err(),
        "tunnel_archive_exists"
    );
    assert!(path.exists());
    assert_eq!(fs::read(destination).unwrap(), b"existing history");
}
#[test]
fn observing_missing_identity_creates_nothing() {
    let temp = root();
    let path = temp.path().join("missing");
    assert!(observe(&path, ORIGIN, "first").unwrap().is_none());
    assert!(!path.exists());
}

#[test]
fn protected_crash_archives_exact_run_and_keeps_receipts_across_clean_stops() {
    let temp = root();
    let root = temp.path().canonicalize().unwrap();
    let binding = [7; 32];
    let (owner, receipts) = restartable(&root, ORIGIN, "first", binding, |_| true).unwrap();
    let previous = observe(&root, ORIGIN, "first").unwrap().unwrap();
    assert_eq!(previous.version, 3);
    let bytes = fs::read(owner.path()).unwrap();
    assert!(receipts.claim("effect-one").unwrap());
    assert!(restartable(&root, ORIGIN, "first", binding, |_| true).is_err());
    drop(receipts);
    drop(owner); // Abrupt shutdown leaves the fence and synced receipt.
    assert!(matches!(
        restartable(&root, ORIGIN, "first", binding, |_| false),
        Err("tunnel_owner_active")
    ));
    let (next, receipts) = restartable(&root, ORIGIN, "first", binding, |_| true).unwrap();
    assert!(!receipts.claim("effect-one").unwrap());
    assert!(receipts.claim("effect-two").unwrap());
    let archived = root
        .join(format!(
            "{}.recovered",
            next.path().file_name().unwrap().to_string_lossy()
        ))
        .join(previous.run_id);
    assert_eq!(fs::read(archived).unwrap(), bytes);
    next.finish().unwrap();
    drop(receipts);
    drop(next);
    assert_eq!(
        RunFence::acquire(&root, ORIGIN, "first").unwrap_err(),
        "tunnel_receipts_required"
    );
    let (last, receipts) = restartable(&root, ORIGIN, "first", binding, |_| true).unwrap();
    assert!(!receipts.claim("effect-one").unwrap());
    assert!(!receipts.claim("effect-two").unwrap());
    last.finish().unwrap();
}

#[test]
fn automatic_recovery_preserves_legacy_missing_corrupt_and_retargeted_history() {
    let temp = root();
    let root = temp.path().canonicalize().unwrap();
    let legacy = RunFence::acquire(&root, ORIGIN, "legacy").unwrap();
    let legacy_path = legacy.path().to_owned();
    drop(legacy);
    assert!(matches!(
        restartable(&root, ORIGIN, "legacy", [1; 32], |_| true),
        Err("tunnel_restart_uncertain")
    ));
    assert!(legacy_path.exists());

    let (owner, receipts) = restartable(&root, ORIGIN, "first", [1; 32], |_| true).unwrap();
    let path = owner.path().to_owned();
    let bytes = fs::read(&path).unwrap();
    assert!(receipts.claim("effect").unwrap());
    drop(receipts);
    drop(owner);
    assert!(matches!(
        restartable(&root, ORIGIN, "first", [2; 32], |_| true),
        Err("tunnel_restart_uncertain")
    ));
    let journal = path.with_extension("receipts");
    let saved = fs::read(&journal).unwrap();
    fs::remove_file(&journal).unwrap();
    assert!(matches!(
        restartable(&root, ORIGIN, "first", [1; 32], |_| true),
        Err("tunnel_receipts_missing")
    ));
    let mut corrupt = saved;
    corrupt[40] ^= 1;
    let mut file = private_options().create_new(true).open(&journal).unwrap();
    file.write_all(&corrupt).unwrap();
    drop(file);
    assert!(matches!(
        restartable(&root, ORIGIN, "first", [1; 32], |_| true),
        Err("tunnel_receipts_invalid")
    ));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read(&journal).unwrap(), corrupt);
}

#[test]
fn interrupted_receipt_append_preserves_previously_synced_admissions() {
    let temp = root();
    let root = temp.path().canonicalize().unwrap();
    let (owner, receipts) = restartable(&root, ORIGIN, "first", [1; 32], |_| true).unwrap();
    let path = owner.path().with_extension("receipts");
    assert!(receipts.claim("synced-effect").unwrap());
    drop(receipts);
    drop(owner);
    let mut file = private_options().open(&path).unwrap();
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(&[0; 17]).unwrap();
    drop(file);
    let (next, receipts) = restartable(&root, ORIGIN, "first", [1; 32], |_| true).unwrap();
    assert!(!receipts.claim("synced-effect").unwrap());
    assert!(receipts.claim("new-effect").unwrap());
    assert_eq!(fs::metadata(path).unwrap().len(), 40 + 2 * 64);
    next.finish().unwrap();
}

#[test]
fn oversized_receipt_history_is_rejected_without_reading_or_replacing_it() {
    let temp = root();
    let root = temp.path().canonicalize().unwrap();
    let (owner, receipts) = restartable(&root, ORIGIN, "first", [1; 32], |_| true).unwrap();
    let path = owner.path().to_owned();
    let bytes = fs::read(&path).unwrap();
    drop(receipts);
    drop(owner);
    let file = private_options()
        .open(&path.with_extension("receipts"))
        .unwrap();
    file.set_len(64_000_041).unwrap();
    drop(file);
    assert!(matches!(
        restartable(&root, ORIGIN, "first", [1; 32], |_| true),
        Err("tunnel_receipts_invalid")
    ));
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

fn restartable(
    root: &Path,
    origin: &str,
    tunnel: &str,
    binding: [u8; 32],
    owner_absent: impl FnOnce(u32) -> bool,
) -> Result<(RunFence, Receipts), &'static str> {
    RunFence::acquire_restartable(root, origin, tunnel, binding, None, |identity| {
        owner_absent(identity.owner_pid)
    })
}

#[test]
fn protected_run_retains_the_exact_process_instance_for_reboot_pid_reuse() {
    let temp = root();
    let root = temp.path().canonicalize().unwrap();
    let (owner, receipts) =
        RunFence::acquire_restartable(&root, ORIGIN, "instance", [1; 32], Some(123), |_| true)
            .unwrap();
    drop(receipts);
    drop(owner);
    let (next, _) =
        RunFence::acquire_restartable(&root, ORIGIN, "instance", [1; 32], Some(456), |old| {
            assert_eq!(old.owner_started, Some(123));
            true
        })
        .unwrap();
    assert_eq!(
        observe(&root, ORIGIN, "instance")
            .unwrap()
            .unwrap()
            .owner_started,
        Some(456)
    );
    next.finish().unwrap();
}
