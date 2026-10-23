use super::*;

fn fixture() -> (PathBuf, StartRequest, ProcessPermissions) {
    let root = std::env::temp_dir().join(format!(
        "omsi-denial-{}-{}",
        std::process::id(),
        crate::lua::next_owner()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = root.join("main.lua");
    std::fs::write(&entry, "return 1").unwrap();
    std::fs::write(root.join("helper.exe"), "helper").unwrap();
    let request = StartRequest {
        permissions: None,
        plugin: "test".into(),
        grants: Arc::new(ProcessGrants::default()),
        entry,
        folder: Some(root.clone()),
        file: "helper.exe".into(),
        args: Vec::new(),
        cwd: None,
        env: Vec::new(),
    };
    (root, request, ProcessPermissions::new(None))
}

#[test]
fn deny_immediately_covers_queued_requests_with_different_content() {
    let (root, request, permissions) = fixture();
    let Authorization::Pending(_, first) = permissions.authorize(&request).unwrap() else {
        panic!("expected prompt");
    };
    let id = permissions.prompt().unwrap().id;
    std::fs::write(&request.entry, "return 2").unwrap();
    let Authorization::Pending(_, second) = permissions.authorize(&request).unwrap() else {
        panic!("expected prompt");
    };
    assert!(permissions.respond(id, ProcessDecision::Deny));
    assert_eq!(first.load(Ordering::Acquire), ProcessDecision::Deny as u8);
    assert_eq!(second.load(Ordering::Acquire), ProcessDecision::Deny as u8);
    assert!(permissions.prompt().is_none());
    // The owner thread need not have processed either decision yet.
    std::fs::write(root.join("helper.exe"), "new helper").unwrap();
    assert!(permissions.authorize(&request).is_err());
    assert!(permissions.prompt().is_none());
    let fresh_session = ProcessPermissions::new(None);
    assert!(matches!(
        fresh_session.authorize(&request),
        Ok(Authorization::Pending(..))
    ));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn denial_budget_is_fail_closed_instead_of_forgetting_new_denials() {
    let (root, request, permissions) = fixture();
    let entry = request.entry.canonicalize().unwrap();
    for n in 0..256 {
        permissions
            .denied
            .lock()
            .unwrap()
            .insert(DenialKey::new(&entry, &format!("{n}.exe")));
    }
    assert!(
        permissions
            .check_denied(&request.entry, "helper.exe")
            .is_err()
    );
    assert!(permissions.authorize(&request).is_err());
    assert!(permissions.prompt().is_none());
    std::fs::remove_dir_all(root).unwrap();
}
