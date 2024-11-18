use crate::{Context, Error};
use poise::serenity_prelude::{ButtonStyle, CreateActionRow, CreateButton};
use rand::{thread_rng, Rng};
use serde::Deserialize;
use std::vec;

#[derive(Deserialize)]
struct ModrinthUser {
    username: String,
    bio: Option<String>,
}

/// Modrinth integration commands
#[poise::command(
    slash_command,
    subcommands("link")
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
                        .get(&format!("https://api.modrinth.com/v2/user/{}", modrinth_id))
                        .send()
                        .await?;

                    if !response.status().is_success() {
                        let error_msg = match response.status().as_u16() {
                            404 => "❌ User not found on Modrinth. Please check your user ID.",
                            429 => "❌ Rate limited by Modrinth API. Please try again in a few minutes.",
                            _ => "❌ Failed to fetch user data from Modrinth. Please try again later.",
                        };
                        msg.edit(ctx, poise::CreateReply::default()
                            .content(error_msg)
                            .components(vec![components.clone()])
                        ).await?;
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
                                settings.save()?;
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
