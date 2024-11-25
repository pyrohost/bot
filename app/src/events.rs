use crate::{commands, Data, Error};
use poise::serenity_prelude::{
    self as serenity, ActivityData, Channel, CreateMessage, GuildChannel, Interaction,
    OnlineStatus, RoleId,
};
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
        serenity::FullEvent::Message { new_message } => {
            if new_message.author.bot {
                return Ok(());
            };
            let mut settings = _data.settings.write().await;
            let old_xp = *settings.get_xp(new_message.author.id).unwrap_or(&0);
            let new_xp = old_xp + 25;
            settings.add_xp(new_message.author.id, 25);
            settings.save()?;
            let old_level = commands::levels::level_from_xp(old_xp);
            let new_level = commands::levels::level_from_xp(new_xp);
            if old_level < new_level {
                let channel = new_message.channel(ctx).await?;
                let Ok(member) = new_message.member(ctx).await else {
                    return Ok(());
                };
                let level_roles: Vec<&RoleId> = settings
                    .get_level_roles()
                    .into_iter()
                    .filter(|a| *a.0 <= new_level && !member.roles.contains(a.1))
                    .map(|a| a.1)
                    .collect();
                // member.add_roles(ctx, level_roles);
                for role in level_roles {
                    member.add_role(ctx, role).await?;
                }
                match channel {
                    Channel::Guild(guild_channel) => {
                        guild_channel
                            .send_message(
                                ctx,
                                CreateMessage::new().content(format!(
                                    "<@{}> just leveled up to level {new_level}!",
                                    new_message.author.id
                                )),
                            )
                            .await?;
                    }
                    _ => {}
                }
            };
        }
        _ => {}
    }
    Ok(())
}
