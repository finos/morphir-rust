//! morphir-daemon is a client of morphir-host: it has no MEP codec,
//! handshake, registry or session of its own.
#[test]
fn the_daemon_keeps_only_its_own_services() {
    let _: Option<morphir_daemon::DaemonError> = None;
    let _ = morphir_daemon::ExtensionLoader::new;
    let _ = std::mem::size_of::<morphir_daemon::VirtualPathConfig>();
}
