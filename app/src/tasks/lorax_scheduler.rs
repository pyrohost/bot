use crate::settings::LoraxState;
use crate::{Data, Error};
use async_trait::async_trait;
use poise::serenity_prelude as serenity;
use tracing::{error, info};

pub struct LoraxSchedulerTask {
    interval: std::time::Duration,
}

impl LoraxSchedulerTask {
    pub fn new() -> Self {
        LoraxSchedulerTask {
            interval: std::time::Duration::from_secs(60), // Run every minute
        }
    }
}

#[async_trait]
impl crate::tasks::Task for LoraxSchedulerTask {
    async fn run(
        &self,
        ctx: &serenity::Context,
        data: Data,
    ) -> Result<(), Error> {
        loop {
            let guild_ids: Vec<_> = {
                let settings = data.settings.read().await;
                settings.guilds.keys().cloned().collect()
            };

            for guild_id in guild_ids {
                if let Err(e) = process_guild_lorax_event(ctx, &data, guild_id).await {
                    error!("Error processing Lorax event for guild {}: {}", guild_id, e);
                }
            }

            // Sleep for the interval duration
            tokio::time::sleep(self.interval).await;
        }
    }
}

async fn process_guild_lorax_event(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: serenity::GuildId,
) -> Result<(), Error> {
    let mut settings = data.settings.write().await;
    let guild_settings = settings.guilds.get_mut(&guild_id).unwrap();

    match &guild_settings.lorax_state {
        LoraxState::Submissions { end_time, .. } => {
            if chrono::Utc::now().timestamp() >= *end_time {
                info!("Submission phase ended for guild {}", guild_id);
                drop(settings);
                crate::commands::lorax::start_voting(ctx, data, guild_id).await?;
            }
        }
        LoraxState::Voting { end_time, .. } => {
            if chrono::Utc::now().timestamp() >= *end_time {
                info!("Voting phase ended for guild {}", guild_id);
                drop(settings);
                crate::commands::lorax::announce_winner(&ctx.http, data, guild_id).await?;
            }
        }
        LoraxState::TieBreaker { end_time, .. } => {
            if chrono::Utc::now().timestamp() >= *end_time {
                info!("Tiebreaker round ended for guild {}", guild_id);
                drop(settings);
                crate::commands::lorax::announce_winner(&ctx.http, data, guild_id).await?;
            }
        }
        LoraxState::Idle => {
            // No action needed
        }
    }

    Ok(())
}