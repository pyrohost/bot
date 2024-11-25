use crate::{Context, Error};
use chrono::{Duration, Utc};
use poise::serenity_prelude::{ButtonStyle, CreateActionRow, CreateButton, RoleId};
use rand::{thread_rng, Rng};
use serde::Deserialize;
use std::{sync::Arc, vec};

const STAFF_ROLE: RoleId = RoleId::new(1104932372467695768);

async fn check_staff_role(ctx: Context<'_>) -> Result<bool, Error> {
    if let Some(_guild_id) = ctx.guild_id() {
        if let Some(member) = ctx.author_member().await {
            return Ok(member.roles.contains(&STAFF_ROLE));
        }
    }
    Ok(false)
}

#[derive(Deserialize)]
struct ModrinthUser {
    username: String,
    bio: Option<String>,
}

/// Modrinth integration commands
#[poise::command(
    slash_command,
    subcommands(
        "link",
        "create_test_server",
        "list_test_servers",
        "delete_test_server"
    )
)]
pub async fn modrinth(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Link your Modrinth account with verification
#[poise::command(slash_command, ephemeral)]
pub async fn link(
    ctx: Context<'_>,
    #[description = "Your Modrinth user ID"] modrinth_id: String,
) -> Result<(), Error> {
    let pool = Arc::clone(&ctx.data().pool);
    let user_id = ctx.author().id;
    let verification_code = format!("pyro-{}", thread_rng().gen::<u32>());

    let components = CreateActionRow::Buttons(vec![
        CreateButton::new("verify")
            .label("I've added the code")
            .style(ButtonStyle::Primary),
        CreateButton::new("retry")
            .label("Get new code")
            .style(ButtonStyle::Secondary),
        CreateButton::new("cancel")
            .label("Cancel")
            .style(ButtonStyle::Danger),
    ]);

    let initial_content = format!(
        "Please add the following code to your Modrinth bio at https://modrinth.com/settings/profile:\n`{}`\nOnce added, click the button below. You can remove the code after verification.",
        verification_code
    );

    let msg = poise::send_reply(
        ctx,
        poise::CreateReply::default()
            .content(&initial_content)
            .components(vec![components.clone()])
            .ephemeral(true),
    )
    .await?;

    loop {
        if let Some(interaction) = msg
            .message()
            .await?
            .await_component_interaction(&ctx.serenity_context().shard)
            .timeout(std::time::Duration::from_secs(120))
            .await
        {
            interaction.defer(&ctx.serenity_context().http).await?;

            match interaction.data.custom_id.as_str() {
                "verify" => {
                    let client = reqwest::Client::new();
                    let response = client
                        .get(format!("https://api.modrinth.com/v2/user/{}", modrinth_id))
                        .send()
                        .await?;

                    if !response.status().is_success() {
                        let error_msg = match response.status().as_u16() {
                            404 => "❌ User not found on Modrinth. Please check your user ID.",
                            429 => "❌ Rate limited by Modrinth API. Please try again in a few minutes.",
                            _ => "❌ Failed to fetch user data from Modrinth. Please try again later.",
                        };
                        msg.edit(
                            ctx,
                            poise::CreateReply::default()
                                .content(error_msg)
                                .components(vec![components.clone()]),
                        )
                        .await?;
                        continue;
                    }

                    let modrinth_user = match response.json::<ModrinthUser>().await {
                        Ok(user) => user,
                        Err(_) => {
                            msg.edit(ctx, poise::CreateReply::default()
                                .content("❌ Failed to parse user data from Modrinth. Please try again later.")
                                .components(vec![components.clone()])
                            ).await?;
                            continue;
                        }
                    };

                    if let Some(bio) = modrinth_user.bio {
                        if bio.contains(&verification_code) {
                            // Success case
                            {
                                let mut settings = ctx.data().settings.write().await;
                                let mut user_settings = settings.get_user_settings(user_id);
                                user_settings.modrinth_id = Some(modrinth_id.clone());
                                settings.set_user_settings(user_id, user_settings);
                                settings.save(&pool).await?;
                            }

                            msg.edit(ctx, poise::CreateReply::default()
                                .content(format!(
                                    "✅ Modrinth account verified and linked to user: [{}](https://modrinth.com/user/{})",
                                    modrinth_user.username, modrinth_id
                                ))
                                .components(vec![])
                            ).await?;
                            break;
                        }
                    }
                    // Failed verification
                    msg.edit(ctx, poise::CreateReply::default()
                        .content(format!(
                            "❌ Verification failed: Code not found in bio.\nCode: `{}`\nPlease add the code and try again, or get a new code.",
                            verification_code
                        ))
                        .components(vec![components.clone()])
                    ).await?;
                }
                "retry" => {
                    let verification_code = format!("pyro-{}", thread_rng().gen::<u32>());
                    msg.edit(ctx, poise::CreateReply::default()
                        .content(format!(
                            "Here's your new verification code:\n`{}`\nPlease add it to your Modrinth bio at https://modrinth.com/settings/profile and click verify. You can remove the code after verification.",
                            verification_code
                        ))
                        .components(vec![components.clone()])
                    ).await?;
                }
                "cancel" => {
                    msg.edit(
                        ctx,
                        poise::CreateReply::default()
                            .content("Operation cancelled.")
                            .components(vec![]),
                    )
                    .await?;
                    break;
                }
                _ => {}
            }
        } else {
            msg.edit(
                ctx,
                poise::CreateReply::default()
                    .content("Timed out waiting for response.")
                    .components(vec![]),
            )
            .await?;
            break;
        }
    }

    Ok(())
}

/// Create a testing server for a Modrinth user
#[poise::command(slash_command)]
pub async fn create_test_server(
    ctx: Context<'_>,
    #[description = "Hours until deletion (max 24, default 4)"] hours: Option<u64>,
    #[description = "Optional: Server name (default: My Testing Server)"] name: Option<String>,
    #[description = "Optional: Create for another user by Modrinth ID"] target_user_id: Option<
        String,
    >,
) -> Result<(), Error> {
    if !check_staff_role(ctx).await? {
        return Err("You need the Staff role to use this command".into());
    }

    let pool = Arc::clone(&ctx.data().pool);

    let hours = hours.unwrap_or(4).min(24);
    let name = name.unwrap_or_else(|| "My Testing Server".to_string());
    let deletion_time = (Utc::now() + Duration::hours(hours as i64)).timestamp();

    let mut settings = ctx.data().settings.write().await;

    // Find the target user's Discord ID and Modrinth ID
    let (discord_id, modrinth_id) = if let Some(target_id) = target_user_id {
        // Admin specified a target user - find their Discord ID from settings
        if let Some((discord_id, _)) = settings
            .user_settings
            .iter()
            .find(|(_, settings)| settings.modrinth_id.as_deref() == Some(&target_id))
        {
            (*discord_id, target_id)
        } else {
            return Err("Target user not found or not linked to Modrinth".into());
        }
    } else {
        // Use the command invoker's linked account
        let user_settings = settings.get_user_settings(ctx.author().id);
        if let Some(id) = user_settings.modrinth_id {
            (ctx.author().id, id)
        } else {
            return Err(
                "You haven't linked your Modrinth account. Use /modrinth link first.".into(),
            );
        }
    };

    // Check server limits
    let mut user_settings = settings.get_user_settings(discord_id);
    if user_settings.testing_servers.len() >= user_settings.max_testing_servers as usize {
        return Err("User has reached their maximum number of testing servers".into());
    }

    // Create server
    let master_key = std::env::var("ARCHON_MASTER_KEY")?;
    let client = reqwest::Client::new();

    let response = client
        .post("https://archon.pyro.host/modrinth/v0/servers/create")
        .header("X-MASTER-KEY", master_key)
        .json(&serde_json::json!({
            "user_id": modrinth_id,
            "name": name,
            "testing": true,
            "specs": {
                "cpu": 2,
                "memory_mb": 1024,
                "swap_mb": 256,
                "storage_mb": 8192
            },
            "source": {
                "loader": "Vanilla",
                "game_version": "latest"
            }
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("Failed to create server: {}", response.status()).into());
    }

    let server_data: serde_json::Value = response.json().await?;
    let server_id = server_data["uuid"].as_str().unwrap().to_string();

    // Update settings
    user_settings
        .testing_servers
        .push(crate::settings::TestingServer {
            server_id: server_id.clone(),
            deletion_time,
        });
    settings.set_user_settings(discord_id, user_settings);
    settings.save(&pool).await?;

    ctx.say(format!(
        "Created testing server for `{}` (ID: [{}](https://modrinth.com/servers/manage/{})). Will be deleted <t:{}:R>.",
        modrinth_id, server_id, server_id, deletion_time
    ))
    .await?;

    Ok(())
}

/// List your testing servers
#[poise::command(slash_command)]
pub async fn list_test_servers(ctx: Context<'_>) -> Result<(), Error> {
    if !check_staff_role(ctx).await? {
        return Err("You need the Staff role to use this command".into());
    }

    let settings = ctx.data().settings.read().await;
    let user_settings = settings.get_user_settings(ctx.author().id);

    if user_settings.testing_servers.is_empty() {
        ctx.say("You don't have any testing servers.").await?;
        return Ok(());
    }

    let servers_list = user_settings
        .testing_servers
        .iter()
        .map(|server| {
            format!(
                "Server ID: [{}](https://modrinth.com/servers/manage/{}) (Expires <t:{}:R>)",
                server.server_id, server.server_id, server.deletion_time
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    ctx.say(format!(
        "Your testing servers ({}/{}): \n{}",
        user_settings.testing_servers.len(),
        user_settings.max_testing_servers,
        servers_list
    ))
    .await?;

    Ok(())
}

async fn server_id_autocomplete<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> impl Iterator<Item = String> + 'a {
    let settings = ctx.data().settings.read().await;
    let user_settings = settings.get_user_settings(ctx.author().id);

    user_settings
        .testing_servers
        .iter()
        .map(|server| server.server_id.clone())
        .filter(|id| id.starts_with(partial))
        .collect::<Vec<_>>()
        .into_iter()
}

/// Delete your testing server early
#[poise::command(slash_command, guild_only)]
pub async fn delete_test_server(
    ctx: Context<'_>,
    #[description = "Server ID"]
    #[autocomplete = server_id_autocomplete]
    server_id: String,
) -> Result<(), Error> {
    if !check_staff_role(ctx).await? {
        return Err("You need the Staff role to use this command".into());
    }

    let pool = Arc::clone(&ctx.data().pool);

    let mut settings = ctx.data().settings.write().await;
    let user_settings = settings.get_user_settings(ctx.author().id);

    // First check if the user owns this server
    user_settings
        .testing_servers
        .iter()
        .find(|s| s.server_id == server_id)
        .ok_or("You don't have a testing server with this ID")?;

    let user_settings = settings
        .user_settings
        .get_mut(&ctx.author().id)
        .ok_or("You don't have any testing servers")?;

    let server_idx = user_settings
        .testing_servers
        .iter()
        .position(|s| s.server_id == server_id)
        .unwrap(); // Safe to unwrap since we already found it

    let master_key = std::env::var("ARCHON_MASTER_KEY")?;
    let client = reqwest::Client::new();

    let response = client
        .post(format!(
            "https://archon.pyro.host/modrinth/v0/servers/{}/delete",
            server_id
        ))
        .header("X-MASTER-KEY", master_key)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("Failed to delete server: {}", response.status()).into());
    }

    user_settings.testing_servers.remove(server_idx);
    settings.save(&pool).await?;

    ctx.say(format!("Successfully deleted server `{}`", server_id))
        .await?;
    Ok(())
}
