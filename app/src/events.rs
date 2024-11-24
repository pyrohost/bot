use crate::{Data, Error};  
use poise::serenity_prelude::{self as serenity, ActivityData, Interaction, OnlineStatus};
use tracing::info;

pub async fn event_handler(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    framework: poise::FrameworkContext<'_, Data, Error>,
    _data: &Data,
) -> Result<(), Error> {
    match event {
        serenity::FullEvent::Ready { .. } => {
            info!("Bot is ready");
            ctx.set_presence(
                Some(ActivityData::watching("our infrastructure")),
                OnlineStatus::Idle,
            );

            
        }
        serenity::FullEvent::InteractionCreate {
            interaction: Interaction::Component(component),
        } => {
            if let Some(_guild_id) = component.guild_id {
                if let Err(e) = crate::commands::lorax::handle_button(
                    ctx,
                    component,
                    framework.user_data.clone(),
                )
                .await
                {
                    tracing::error!("Error handling button: {}", e);
                }
            }
        }
        _ => {}
    }
    Ok(())
}
