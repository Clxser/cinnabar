use super::*;

#[test]
fn failed_no_clobber_publication_keeps_original_and_retires_owned_staging() {
    let folder = tempfile::tempdir().unwrap();
    let chosen = folder.path().join("chosen.mp4");
    fs::write(&chosen, b"original").unwrap();
    let staged = staging(folder.path()).unwrap();
    let temporary = staged.to_path_buf();
    fs::write(&staged, b"replacement").unwrap();
    assert!(publish(staged, &Destination::File(chosen.clone())).is_err());
    assert_eq!(fs::read(chosen).unwrap(), b"original");
    assert!(!temporary.exists());
}

#[test]
fn directory_publication_reserves_unique_names_without_losing_either_export() {
    let folder = tempfile::tempdir().unwrap();
    let mut outputs = Vec::new();
    for pixels in [b"one", b"two"] {
        let staged = staging(folder.path()).unwrap();
        fs::write(&staged, pixels).unwrap();
        let output = publish(staged, &Destination::Directory(folder.path().into())).unwrap();
        assert_eq!(fs::read(&output.path).unwrap(), pixels);
        outputs.push(output.path);
    }
    assert_ne!(outputs[0], outputs[1]);
}
