use super::parse;
#[test]
fn frontend_requires_one_fixed_socket_and_rejects_ignored_overrides() {
    assert!(
        parse(&["--supervisor-socket".into(), "/fixed/front.sock".into()])
            .unwrap()
            .is_some()
    );
    assert!(
        parse(&["--transport".into(), "stdio".into()])
            .unwrap()
            .is_none()
    );
    for args in [
        vec!["--supervisor-socket"],
        vec!["--supervisor-socket=/fixed"],
        vec!["--supervisor-socket", "/fixed", "--data-dir", "/foreign"],
        vec!["--profile", "all", "--supervisor-socket", "/fixed"],
    ] {
        assert!(parse(&args.into_iter().map(Into::into).collect::<Vec<_>>()).is_err());
    }
}
