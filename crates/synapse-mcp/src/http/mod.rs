mod auth;
mod session;
mod transport;

pub async fn serve(bind: &str, allow_non_loopback: bool) -> anyhow::Result<std::process::ExitCode> {
    transport::serve(bind, allow_non_loopback).await
}
