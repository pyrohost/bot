use crate::commands::lorax::{announce_winner, start_voting};
use crate::settings::LoraxState;
use crate::{Data, Error};
use async_trait::async_trait;
use chrono::Utc;
use poise::serenity_prelude as serenity;
use tokio::time::{sleep, Duration};
use tracing::{error, info};

use super::TaskHandler;

pub struct LoraxSchedulerTask;

impl LoraxSchedulerTask {
    pub fn new() -> Self {
        Self
    }

    async fn recover_events(
        ctx: &serenity::Context,
        data: &Data,
        guild_id: serenity::GuildId,
        state: &LoraxState,
    ) -> Result<(), Error> {
        let now = Utc::now().timestamp();
        
        match state {
            LoraxState::Submissions { end_time, .. } if now < *end_time => {
                let delay = Duration::from_secs((*end_time - now) as u64);
                let data = data.clone();
                let ctx = ctx.clone();
                
                tokio::spawn(async move {
                    sleep(delay).await;
                    if let Err(e) = start_voting(&ctx, &data, guild_id, 60).await {
                        error!("Failed to start voting after recovery: {}", e);
                    }
                });
            }
            LoraxState::Voting { end_time, .. } | LoraxState::TieBreaker { end_time, .. } if now < *end_time => {
                let delay = Duration::from_secs((*end_time - now) as u64);
                let data = data.clone();
                let http = ctx.http.clone();
                
                tokio::spawn(async move {
                    sleep(delay).await;
                    if let Err(e) = announce_winner(&http, &data, guild_id).await {
                        error!("Failed to announce winner after recovery: {}", e);
                    }
                });
            }
            _ => {}
        }
        Ok(())
    }

    // this shit's nasty 
    // -ellie
    pub async fn schedule_state_transition(
        &self,
        ctx: &serenity::Context,
        data: &Data,
        guild_id: serenity::GuildId,
        state: LoraxState,
        role_id: Option<serenity::RoleId>,
    ) -> Result<(), Error> {
        match &state {
            LoraxState::Submissions { end_time, .. } => {
                let delay = Duration::from_secs((*end_time - Utc::now().timestamp()) as u64);
                let data = data.clone();
                let ctx = ctx.clone();
                
                tokio::spawn(async move {
                    sleep(delay).await;
                    if let Err(e) = start_voting(&ctx, &data, guild_id, 60).await {
                        error!("Failed to start voting phase: {}", e);
                    }
                });
            }
            LoraxState::Voting { end_time, .. } | LoraxState::TieBreaker { end_time, .. } => {
                let delay = Duration::from_secs((*end_time - Utc::now().timestamp()) as u64);
                let data = data.clone();
                let http = ctx.http.clone();
                
                tokio::spawn(async move {
                    sleep(delay).await;
                    if let Err(e) = announce_winner(&http, &data, guild_id).await {
                        error!("Failed to announce winner: {}", e);
                    }
                });
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn schedule_next_phase(
        ctx: &serenity::Context,
        data: &Data,
        guild_id: serenity::GuildId,
        end_time: i64,
        current_state: LoraxState,
    ) -> Result<(), Error> {
        let delay = Duration::from_secs((end_time - Utc::now().timestamp()) as u64);
        let data = data.clone();
        let ctx = ctx.clone();

        tokio::spawn(async move {
            sleep(delay).await;
            match current_state {
                LoraxState::Submissions { .. } => {
                    if let Err(e) = start_voting(&ctx, &data, guild_id, 60).await {
                        error!("Failed to start voting phase: {}", e);
                    }
                }
                LoraxState::Voting { .. } | LoraxState::TieBreaker { .. } => {
                    if let Err(e) = announce_winner(&ctx.http, &data, guild_id).await {
                        error!("Failed to announce winner: {}", e);
                    }
                }
                _ => {}
            }
        });

        Ok(())
    }
}

#[async_trait]
impl TaskHandler for LoraxSchedulerTask {
    fn name(&self) -> &'static str {
        "lorax_scheduler"
    }

    async fn run(&mut self, ctx: &serenity::Context, data: Data) -> Result<(), Error> {
        info!("Recovering lorax events after restart...");
        
        let settings = data.settings.read().await;
        for (&guild_id, guild_settings) in settings.guilds.iter() {
            if let Err(e) = Self::recover_events(ctx, &data, guild_id, &guild_settings.lorax_state).await {
                error!("Failed to recover lorax events for guild {}: {}", guild_id, e);
            }
        }
        
        Ok(())
    }
}