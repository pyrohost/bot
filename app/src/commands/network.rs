use crate::{Context, Error};
use poise::serenity_prelude::{
    self as serenity, ButtonStyle, Channel, ChannelType, CreateActionRow, CreateButton,
    CreateChannel,
};
use std::vec;

async fn create_stat_channel(
    ctx: &Context<'_>,
    guild_id: serenity::GuildId,
    name: &str,
    category_id: serenity::ChannelId,
) -> Result<Channel, Error> {
    let guild_channel = guild_id
        .create_channel(
            &ctx.serenity_context().http,
            CreateChannel::new(format!("{} Loading...", name))
                .kind(ChannelType::Voice)
                .category(category_id),
        )
        .await?;
    Ok(Channel::Guild(guild_channel))
}

#[poise::command(slash_command, required_permissions = "MANAGE_CHANNELS")]
pub async fn setup_stats(
    ctx: Context<'_>,
    #[description = "Category for stats"] channel: Channel,
) -> Result<(), Error> {
    let channel_id = channel.id();
    
    if channel.clone().category().is_none() {
        poise::say_reply(ctx, "This command must be used in a category").await?;
        return Ok(());
    }

    let guild_id = ctx
        .guild_id()
        .ok_or("This command must be used in a server")?;

    // Find existing channels in category
    let channels = guild_id.channels(&ctx.serenity_context().http).await?;
    let existing_channels: Vec<_> = channels
        .values()
        .filter(|c| c.parent_id == Some(channel_id))
        .collect();

    if !existing_channels.is_empty() {
        let components = CreateActionRow::Buttons(vec![
            CreateButton::new("yes")
                .label("Yes")
                .style(ButtonStyle::Danger),
            CreateButton::new("no")
                .label("No")
                .style(ButtonStyle::Primary),
        ]);

        let builder = poise::CreateReply::default()
            .content(format!(
                "This will delete {} existing channels in the {} category. Continue?",
                existing_channels.len(),
                channel
            ))
            .components(vec![components]);

        let msg = poise::send_reply(ctx, builder).await?;

        if let Some(interaction) = msg
            .message()
            .await?
            .await_component_interaction(&ctx.serenity_context().shard)
            .timeout(std::time::Duration::from_secs(60))
            .await
        {
            interaction.defer(&ctx.serenity_context().http).await?;

            match interaction.data.custom_id.as_str() {
                "yes" => {
                    for channel in existing_channels {
                        channel.delete(&ctx.serenity_context().http).await?;
                    }
                }
                "no" => {
                    msg.edit(
                        ctx,
                        poise::CreateReply::default().content("Operation cancelled."),
                    )
                    .await?;
                    return Ok(());
                }
                _ => {}
            }
        } else {
            msg.edit(
                ctx,
                poise::CreateReply::default().content("Timed out waiting for response."),
            )
            .await?;
            return Ok(());
        }
    }

    let channel_configs = vec![
        ("🖥️ Nodes", "nodes_channel"),
        ("🧠 Memory", "memory_channel"),
        ("💾 Storage", "storage_channel"),
        ("🌐 Network", "network_channel"),
        ("📊 Bandwidth", "network_total_channel"),
    ];

    let mut created_channels = Vec::new();
    for (name, _) in &channel_configs {
        match create_stat_channel(&ctx, guild_id, name, channel_id).await {
            Ok(new_channel) => created_channels.push((name, new_channel)),
            Err(e) => {
                // Cleanup on error
                for (_, ch) in created_channels {
                    let _ = ch.delete(&ctx.serenity_context().http).await;
                }
                return Err(format!("Failed to create {} channel: {}", name, e).into());
            }
        }
    }

    {
        let mut settings = ctx.data().settings.write().await;
        let mut guild_settings = settings.get_guild_settings(guild_id);
        guild_settings.stats_category = Some(channel_id);

        for ((_, channel), (_, field_name)) in created_channels.iter().zip(channel_configs.iter()) {
            match *field_name {
                "nodes_channel" => guild_settings.nodes_channel = Some(channel.id()),
                "network_channel" => guild_settings.network_channel = Some(channel.id()),
                "storage_channel" => guild_settings.storage_channel = Some(channel.id()),
                "memory_channel" => guild_settings.memory_channel = Some(channel.id()),
                "network_total_channel" => guild_settings.network_total_channel = Some(channel.id()),
                _ => {}
            }
        }

        settings.set_guild_settings(guild_id, guild_settings);
        settings.save()?;
    }

    poise::say_reply(ctx, "✅ Stats channels setup complete!").await?;
    Ok(())
}
