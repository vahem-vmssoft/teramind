use teramind_ipc::proto::{Request, Response};

pub async fn run() -> anyhow::Result<()> {
    match crate::ipc::request(Request::Shutdown, 1500).await {
        Ok(Response::Ok) => {
            println!("teramind: stop requested");
            Ok(())
        }
        Ok(other) => {
            eprintln!("unexpected: {other:?}");
            Ok(())
        }
        Err(_) => {
            println!("teramind: daemon already stopped");
            Ok(())
        }
    }
}
