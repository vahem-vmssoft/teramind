use teramind_db::pg_supervisor::PgSupervisor;
use tempfile::tempdir;

#[tokio::test]
async fn supervisor_starts_and_stops_embedded_pg() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().to_path_buf(), "teramind_test")
        .await
        .unwrap();
    let _opts = sup.connect_options();
    sup.shutdown().await.unwrap();
}
