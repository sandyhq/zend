//! android-app spike S1 (throwaway): the engine alone, offline (no edge
//! token, no WorkOS), to see whether it starts and serves its IPC on
//! Android. `engine-probe <data-dir> [port]`.

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let data_dir = std::path::PathBuf::from(args.next().unwrap_or_else(|| "/data/local/tmp/zend-probe".into()));
    let ipc_port = args.next().and_then(|p| p.parse().ok()).unwrap_or(27699);
    let config = zend_engine::EngineConfig {
        data_dir,
        edge_url: std::env::var("ZEND_EDGE_URL").unwrap_or_else(|_| "http://127.0.0.1:9".into()),
        edge_token: None,
        ipc_port,
        default_harness: zend_engine::HarnessId::Mock,
        org_id: None,
        workos_client_id: None,
    };
    eprintln!("engine-probe: starting on 127.0.0.1:{ipc_port}");
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(zend_engine::Engine::new(config).run())
}
