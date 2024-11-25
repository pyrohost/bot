use crate::{Context, Error};
use poise::serenity_prelude::{GuildChannel, Role};
use std::vec;

pub fn level_from_xp(xp: u32) -> u32 {
    ((xp as f32 / 100.0).powf(0.65)).floor() as u32
}

pub fn xp_to_next_level(xp: u32) -> u32 {
    let current_level = level_from_xp(xp);
    let next_level = current_level + 1;
    let xp_for_next_level = ((next_level as f32).powf(1.0 / 0.65)) * 100.0;
    (xp_for_next_level - xp as f32).round() as u32 + 1
}

#[poise::command(
    slash_command,
    subcommands("add_role", "remove_role", "remove_level", "set_channel", "get_level")
)]
pub async fn levels(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn add_role(ctx: Context<'_>, role: Role, level: u32) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let mut settings = ctx.data().settings.write().await;
    settings.add_level_role(role.id, level);
    settings.save()?;
    ctx.reply(format!("Added role `{}` for level `{}`!", role.name, level))
        .await?;
    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn remove_role(ctx: Context<'_>, role: Role) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let mut settings = ctx.data().settings.write().await;
    settings.remove_role_by_role_id(role.id);
    settings.save()?;
    ctx.reply(format!(
        "Removed the level milestone for role `{}`!",
        role.name
    ))
    .await?;
    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn remove_level(ctx: Context<'_>, level: u32) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let mut settings = ctx.data().settings.write().await;
    settings.remove_role_by_level(level);
    settings.save()?;
    ctx.reply(format!("Removed the level milestone for level {level}!"))
        .await?;
    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn set_channel(ctx: Context<'_>, channel: GuildChannel) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let mut settings = ctx.data().settings.write().await;
    settings.set_level_channel(channel.id);
    settings.save()?;
    ctx.reply(format!("Set channel to <#{}>!", channel.id))
        .await?;
    Ok(())
}

#[poise::command(slash_command)]
pub async fn get_level(ctx: Context<'_>) -> Result<(), Error> {
    let mut settings = ctx.data().settings.write().await;
    let Some(xp) = settings.get_xp(ctx.author().id) else {
        ctx.reply("Couldn't determine your XP/level.").await?;
        return Ok(());
    };
    let xp_left = xp_to_next_level(*xp);
    let level = level_from_xp(*xp);
    ctx.reply(format!(
        "You are at level {level}, with {xp_left} XP left until level {}.",
        level + 1
    ))
    .await?;
    Ok(())
}
