mod commands;
mod error;
mod events;
mod metrics;
mod settings;
mod tasks;

use events::event_handler;
use poise::serenity_prelude as serenity;
use settings::Settings;
use std::sync::Arc;
use tokio::sync::RwLock;
use tasks::TaskManager;
use tasks::stats_updater::StatsUpdaterTask;
use tasks::lorax_scheduler::LoraxSchedulerTask;

#[derive(Clone)]
pub struct Data {
    pub settings: Arc<RwLock<Settings>>,
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Data, Error>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    let token = std::env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN not set");
    let intents = serenity::GatewayIntents::non_privileged()
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_MEMBERS;

    let settings = Arc::new(RwLock::new(Settings::load()?));
    
    let mut task_manager = TaskManager::new();
    task_manager.register_task(StatsUpdaterTask::new());
    task_manager.register_task(LoraxSchedulerTask::new());

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                commands::lorax::lorax(),
                commands::lorax::setup(),
                commands::modrinth::modrinth(),
                commands::query::query(),
                commands::network::setup_stats(),
            ],
            event_handler: |ctx, event, framework, data| {
                Box::pin(event_handler(ctx, event, framework, data))
            },
            ..Default::default()
        })
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                let data = Data { settings: settings.clone() };
                
                // Run tasks after framework setup
                task_manager.run_all(ctx, data.clone()).await;
                
                Ok(data)
            })
        })
        .build();

    serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await?
        .start()
        .await?;

    Ok(())
}
