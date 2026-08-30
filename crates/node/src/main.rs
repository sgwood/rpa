#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ai_rpa_node::cli::run().await
}
