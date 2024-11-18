use crate::metrics::MetricsClient;
use crate::settings::LoraxState;
use crate::{Context, Data, Error};
use chrono::{Duration, Utc};
use poise::serenity_prelude::{
    self as serenity, ButtonStyle, ChannelId, Color, ComponentInteraction, CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter, CreateInteractionResponseMessage, CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption, RoleId, UserId
};
use poise::CreateReply;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

/// Lorax tree naming system
#[poise::command(
    slash_command,
    subcommands("set_role", "set_channel", "start", "submit", "vote", "list", "cancel", "extend", "status", "leaderboard", "edit"),
)]
pub async fn lorax(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Set the Lorax announcement channel
#[poise::command(slash_command, required_permissions = "MANAGE_CHANNELS")]
pub async fn set_channel(
    ctx: Context<'_>,
    #[description = "Channel for tree naming announcements"] channel: ChannelId,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    {
        let mut settings = ctx.data().settings.write().await;
        let mut guild_settings = settings.get_guild_settings(guild_id);
        guild_settings.lorax_channel = Some(channel);
        settings.set_guild_settings(guild_id, guild_settings);
        settings.save()?;
    }

    ctx.say(format!(
        "✅ Set Lorax announcement channel to <#{}>",
        channel
    ))
    .await?;

    Ok(())
}

/// Set the Lorax role to ping
#[poise::command(slash_command, required_permissions = "MANAGE_ROLES")]
pub async fn set_role(
    ctx: Context<'_>,
    #[description = "Role to ping for tree naming events"] role: RoleId,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    {
        let mut settings = ctx.data().settings.write().await;
        let mut guild_settings = settings.get_guild_settings(guild_id);
        guild_settings.lorax_role = Some(role);
        settings.set_guild_settings(guild_id, guild_settings);
        settings.save()?;
    }

    ctx.say(format!("✅ Set Lorax role to <@&{}>", role))
        .await?;

    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn start(
    ctx: Context<'_>,
    #[description = "Submission duration in minutes"] submission_duration: Option<u64>,
    #[description = "Voting duration in minutes"] voting_duration: Option<u64>,
) -> Result<(), Error> {
    let submission_duration = submission_duration.unwrap_or(60);
    let voting_duration = voting_duration.unwrap_or(60);
    let guild_id = ctx.guild_id().unwrap();

    let mut settings = ctx.data().settings.write().await;
    let guild_settings = settings.get_guild_settings(guild_id);

    if guild_settings.lorax_channel.is_none() || guild_settings.lorax_role.is_none() {
        ctx.say("Lorax channel or role not set. Please set them before starting.")
            .await?;
        return Ok(());
    }

    if let LoraxState::Idle = guild_settings.lorax_state {
        let end_time = Utc::now().timestamp() + (submission_duration * 60) as i64;

        let role_id = guild_settings.lorax_role.unwrap();
        let announcement = format!(
            "<@&{}> Submissions for tree names are now open! 🌱\n\nSubmissions close {}\nUse `/lorax submit` to submit your tree name\nUse `/lorax list` to view current submissions",
            role_id,
            discord_timestamp(end_time, TimestampStyle::ShortDateTime)
        );

        // Announce the start of submissions
        let channel_id = guild_settings.lorax_channel.unwrap();
        let role_id = guild_settings.lorax_role.unwrap();
        let announcement_msg = channel_id.say(&ctx, announcement).await?;

        // Update LoraxState to Submissions
        settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Submissions {
            end_time,
            message_id: announcement_msg.id,
            submissions: HashMap::new(),
        };
        settings.save()?;

        // Schedule transition to voting
        let data = ctx.data().clone();
        let ctx_clone = ctx.serenity_context().clone();
        tokio::spawn(async move {
            tokio::time::sleep(
                Duration::seconds(submission_duration as i64 * 60)
                    .to_std()
                    .unwrap(),
            )
            .await;
            if let Err(e) = start_voting(&ctx_clone, &data, guild_id, voting_duration).await {
                tracing::error!("Failed to start voting: {}", e);
            }
        });

        ctx.say("Lorax event started!").await?;
    } else {
        ctx.say("A Lorax event is already in progress.").await?;
    }

    Ok(())
}

// Helper function to start voting phase
async fn start_voting(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: serenity::GuildId,
    voting_duration: u64,
) -> Result<(), Error> {
    let mut settings = data.settings.write().await;
    let state = &settings.guilds.get(&guild_id).unwrap().lorax_state;

    if let LoraxState::Submissions { submissions, .. } = state {
        if submissions.is_empty() {
            // No submissions, end the event
            settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Idle;
            settings.save()?;
            return Ok(());
        }

        let end_time = Utc::now().timestamp() + (voting_duration * 60) as i64;
        let options = submissions.values().cloned().collect::<Vec<_>>();
        let submission_count = options.len();

        let channel_id = settings.guilds.get(&guild_id).unwrap().lorax_channel.unwrap();
        let role_id = settings.guilds.get(&guild_id).unwrap().lorax_role.unwrap();
        
        let announcement = format!(
            "<@&{}> Voting is now open! 🗳️\n\n🌳 {} tree names have been submitted\nVoting closes {}\n\nUse `/lorax vote` to view and vote for submissions\n**Note:** You cannot vote for your own submission",
            role_id,
            submission_count,
            discord_timestamp(end_time, TimestampStyle::ShortDateTime)
        );
        
        let announcement_msg = channel_id.say(ctx, announcement).await?;

        settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Voting {
            end_time,
            message_id: announcement_msg.id,
            options,
            votes: HashMap::new(),
        };
        settings.save()?;

        // Schedule end of voting
        let data_clone = data.clone();
        let ctx_clone = ctx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(
                Duration::seconds(voting_duration as i64 * 60)
                    .to_std()
                    .unwrap(),
            )
            .await;
            if let Err(e) = announce_winner(&ctx_clone, &data_clone, guild_id).await {
                tracing::error!("Failed to announce winner: {}", e);
            }
        });
    }

    Ok(())
}

// Helper function to announce the winner
async fn announce_winner(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: serenity::GuildId,
) -> Result<(), Error> {
    let mut settings = data.settings.write().await;
    // Remove unused guild_settings variable and use direct access
    let guild = settings.guilds.get(&guild_id).unwrap();

    if let LoraxState::Voting { options, votes, .. } = &guild.lorax_state {
        // Tally votes
        let mut vote_counts = HashMap::new();
        for &choice in votes.values() {
            *vote_counts.entry(choice).or_insert(0) += 1;
        }

        let total_votes = votes.len();

        if let Some((&winning_option, count)) = vote_counts.iter().max_by_key(|&(_, count)| count) {
            let winning_tree = &options[winning_option];
            let submissions =
                if let LoraxState::Submissions { submissions, .. } = &guild.lorax_state {
                    submissions
                } else {
                    &HashMap::new()
                };
            let winner_mention = submissions
                .iter()
                .find(|(_, tree)| *tree == winning_tree)
                .map_or("Unknown User".to_string(), |(user_id, _)| format!("<@{}>", user_id));

            let channel_id = guild.lorax_channel.unwrap();
            let role_id = guild.lorax_role.unwrap();
            let vote_distribution = options
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let votes = *vote_counts.get(&i).unwrap_or(&0);
                    let percentage = if total_votes > 0 {
                        (votes * 100) / total_votes
                    } else {
                        0
                    };
                    format!(
                        "{} {} - {} votes ({}%)",
                        if i == winning_option { "👑" } else { "🌳" },
                        *name,
                        votes,
                        percentage
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            let announcement = format!(
                "<@&{}> The voting has concluded!\n\n**Winner: {}** 🎉\nSubmitted by: {}\nReceived {} votes\n\nFinal Results:\n{}",
                role_id,
                winning_tree,
                winner_mention,
                count,
                vote_distribution
            );
            channel_id.say(ctx, announcement).await?;
        } else {
            // No votes, announce no winner
            let channel_id = guild.lorax_channel.unwrap();
            channel_id
                .say(ctx, "No votes were cast. No tree was selected.")
                .await?;
        }

        // Reset Lorax state
        settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Idle;
        settings.save()?;
    }

    Ok(())
}

// Helper function to format Discord timestamps
fn discord_timestamp(time: i64, style: TimestampStyle) -> String {
    format!("<t:{}:{}>", time, style.as_str())
}

enum TimestampStyle {
    ShortTime,      // t - 9:41 PM
    LongTime,       // T - 9:41:30 PM
    ShortDate,      // d - 06/09/2023
    LongDate,       // D - June 9, 2023
    ShortDateTime,  // f - June 9, 2023 9:41 PM
    LongDateTime,   // F - Friday, June 9, 2023 9:41 PM
    Relative,       // R - 2 months ago
}

impl TimestampStyle {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ShortTime => "t",
            Self::LongTime => "T",
            Self::ShortDate => "d",
            Self::LongDate => "D",
            Self::ShortDateTime => "f",
            Self::LongDateTime => "F",
            Self::Relative => "R",
        }
    }
}

// Helper function to validate tree name
fn validate_tree_name(name: &str) -> Result<(), &'static str> {
    if name.len() < 3 || name.len() > 20 {
        return Err("Tree name must be between 3 and 20 characters long.");
    }
    if !name.chars().all(|c| c.is_ascii_lowercase()) {
        return Err("Only lowercase ASCII letters are allowed, with no spaces.");
    }
    Ok(())
}

/// Cancel ongoing Lorax event
#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn cancel(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let mut settings = ctx.data().settings.write().await;
    
    match &settings.guilds.get(&guild_id).unwrap().lorax_state {
        LoraxState::Idle => {
            ctx.say("No active Lorax event to cancel.").await?;
            return Ok(());
        }
        _ => {
            let channel_id = settings.guilds.get(&guild_id).unwrap().lorax_channel.unwrap();
            channel_id.say(&ctx, "🚫 The current Lorax event has been cancelled by an administrator.").await?;
            settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Idle;
            settings.save()?;
            ctx.say("✅ Lorax event cancelled.").await?;
        }
    }
    Ok(())
}

/// Extend current phase duration
#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn extend(
    ctx: Context<'_>,
    #[description = "Additional minutes"] duration: u64,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let mut settings = ctx.data().settings.write().await;
    
    match &mut settings.guilds.get_mut(&guild_id).unwrap().lorax_state {
        LoraxState::Idle => {
            ctx.say("No active Lorax event to extend.").await?;
        }
        LoraxState::Submissions { end_time, .. } |
        LoraxState::Voting { end_time, .. } => {
            *end_time += duration as i64 * 60;
            let new_end = *end_time;
            
            // Clone necessary values before dropping mutable borrow
            let channel = settings.guilds.get(&guild_id).unwrap().lorax_channel;
            drop(settings);

            if let Some(channel_id) = channel {
                channel_id.say(&ctx, 
                    format!("⏰ The current phase has been extended by {} minutes. New end time: {}", 
                        duration, discord_timestamp(new_end, TimestampStyle::ShortDateTime))).await?;
                ctx.say("✅ Successfully extended the duration.").await?;
            }
        }
    }
    Ok(())
}

#[poise::command(slash_command, ephemeral)]
pub async fn submit(
    ctx: Context<'_>,
    #[description = "Your tree name (letters only, no spaces)"] name: String,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let user_id = ctx.author().id;
    let tree_name = name.to_lowercase();

    if let Err(msg) = validate_tree_name(&tree_name) {
        ctx.say(msg).await?;
        return Ok(());
    }

    if tree_name.len() < 3 || tree_name.len() > 20 {
        ctx.say("Tree name must be between 3 and 20 characters long.")
            .await?;
        return Ok(());
    }

    if !tree_name.chars().all(|c| c.is_ascii_lowercase()) {
        ctx.say("Invalid tree name. Only lowercase ASCII letters are allowed, with no spaces.")
            .await?;
        return Ok(());
    }

    // Check for existing tree names using metrics API
    let metrics_client = MetricsClient::new();
    let existing_trees = metrics_client.fetch_existing_trees().await?;
    if existing_trees.contains(&tree_name) {
        ctx.say("This tree name is already in use.").await?;
        return Ok(());
    }

    let mut settings = ctx.data().settings.write().await;

    if let LoraxState::Submissions { submissions, .. } =
        &mut settings.guilds.get_mut(&guild_id).unwrap().lorax_state
    {
        // Check if name is already submitted in current session
        if submissions.values().any(|n| n == &tree_name) {
            ctx.say("This tree name has already been submitted in this session.")
                .await?;
            return Ok(());
        }

        if let Entry::Vacant(entry) = submissions.entry(user_id) {
             entry.insert(tree_name.clone());
             ctx.say("Your tree name has been submitted!").await?;
         } else {
             ctx.say("You have already submitted a tree name.").await?;
         }

        settings.save()?;
    } else {
        ctx.say("Submissions are not currently open.").await?;
    }

    Ok(())
}

#[poise::command(slash_command, ephemeral)]
pub async fn vote(
    ctx: Context<'_>,
    #[description = "Search for a specific tree name (optional)"] search: Option<String>,
    #[description = "Page number"] page: Option<u32>,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let user_id = ctx.author().id;
    let settings = ctx.data().settings.read().await;
    let page = page.unwrap_or(1).max(1);
    const ITEMS_PER_PAGE: usize = 10;

    if let Some(guild) = settings.guilds.get(&guild_id) {
        if let LoraxState::Voting {
            options,
            votes,
            end_time,
            ..
        } = &guild.lorax_state
        {
            if Utc::now().timestamp() > *end_time {
                ctx.say("Voting period has ended.").await?;
                return Ok(());
            }

            // Filter options based on search term
            let filtered_options: Vec<_> = if let Some(ref search) = search {
                options
                    .iter()
                    .enumerate()
                    .filter(|(_, name)| name.contains(&search.to_lowercase()))
                    .collect()
            } else {
                options.iter().enumerate().collect()
            };

            // Calculate pagination
            let total_pages = (filtered_options.len() + ITEMS_PER_PAGE - 1) / ITEMS_PER_PAGE;
            let start_idx = (page as usize - 1) * ITEMS_PER_PAGE;
            let end_idx = start_idx + ITEMS_PER_PAGE;
            let page_options = &filtered_options[start_idx.min(filtered_options.len())..end_idx.min(filtered_options.len())];

            let mut components = Vec::new();
            
            // Create select menu for voting
            let select_options = page_options
                .iter()
                .map(|(i, name)| {
                    CreateSelectMenuOption::new(
                        *name,
                        i.to_string(),
                    ).default_selection(votes.get(&user_id) == Some(i))
                })
                .collect::<Vec<_>>();

            if !select_options.is_empty() {
                components.push(CreateActionRow::SelectMenu(
                    CreateSelectMenu::new("vote_select", CreateSelectMenuKind::String { options: select_options })
                        .placeholder("Select a tree name to vote for")
                ));
            }

            // Add navigation buttons if needed
            if total_pages > 1 {
                let mut nav_buttons = Vec::new();
                if page > 1 {
                    nav_buttons.push(
                        CreateButton::new(format!("page_{}", page - 1))
                            .label("Previous")
                            .style(ButtonStyle::Secondary),
                    );
                }
                if page < total_pages as u32 {
                    nav_buttons.push(
                        CreateButton::new(format!("page_{}", page + 1))
                            .label("Next")
                            .style(ButtonStyle::Secondary),
                    );
                }
                if !nav_buttons.is_empty() {
                    components.push(CreateActionRow::Buttons(nav_buttons));
                }
            }

            let current_vote = votes.get(&user_id).map(|&idx| &options[idx]);
            let header = format!(
                "🗳️ Voting ends {}\n{}\nPage {}/{}\n{} total options{}",
                discord_timestamp(*end_time, TimestampStyle::ShortDateTime),
                if let Some(vote) = current_vote {
                    format!("Your current vote: {}", vote)
                } else {
                    "You haven't voted yet".to_string()
                },
                page,
                total_pages.max(1),
                filtered_options.len(),
                if let Some(search_term) = &search {
                    format!(" (filtered by \"{}\")", search_term)
                } else {
                    String::new()
                }
            );

            ctx.send(
                CreateReply::default()
                    .content(header)
                    .components(components)
                    .ephemeral(true),
            )
            .await?;
        } else {
            ctx.say("Voting is not currently open.").await?;
        }
    }

    Ok(())
}

/// Show voting leaderboard
#[poise::command(slash_command)]
pub async fn leaderboard(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let settings = ctx.data().settings.read().await;

    if let Some(guild) = settings.guilds.get(&guild_id) {
        if let LoraxState::Voting { options, votes, .. } = &guild.lorax_state {
            let mut vote_counts: HashMap<usize, usize> = HashMap::new();
            for &choice in votes.values() {
                *vote_counts.entry(choice).or_insert(0) += 1;
            }

            let mut rankings: Vec<_> = vote_counts.iter().collect();
            rankings.sort_by_key(|&(_, count)| std::cmp::Reverse(*count));

            let leaderboard = rankings
                .iter()
                .take(10)
                .enumerate()
                .map(|(i, (&option_idx, &votes))| {
                    format!(
                        "{} {} - {} votes",
                        match i {
                            0 => "🥇",
                            1 => "🥈",
                            2 => "🥉",
                            _ => "🌳",
                        },
                        options[option_idx],
                        votes
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            ctx.send(
                CreateReply::default()
                    .embed(CreateEmbed::default()
                        .title("🏆 Tree Name Leaderboard")
                        .description(leaderboard)
                        .footer(CreateEmbedFooter::new(
                            format!("Total votes: {}", votes.len())
                        ))
                        .timestamp(Utc::now()))
            ).await?;
        } else {
            ctx.say("No active voting session.").await?;
        }
    }

    Ok(())
}

/// Edit your submitted tree name
#[poise::command(slash_command, ephemeral)]
pub async fn edit(
    ctx: Context<'_>,
    #[description = "New tree name"] new_name: String,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let user_id = ctx.author().id;
    let tree_name = new_name.to_lowercase();

    if let Err(msg) = validate_tree_name(&tree_name) {
        ctx.say(msg).await?;
        return Ok(());
    }

    let mut settings = ctx.data().settings.write().await;

    if let LoraxState::Submissions { submissions, .. } =
        &mut settings.guilds.get_mut(&guild_id).unwrap().lorax_state
    {
        if !submissions.contains_key(&user_id) {
            ctx.say("You haven't submitted a tree name yet.").await?;
            return Ok(());
        }

        if submissions.values().any(|n| n == &tree_name) {
            ctx.say("This tree name has already been submitted.").await?;
            return Ok(());
        }

        submissions.insert(user_id, tree_name.clone());
        settings.save()?;
        ctx.say("✅ Your tree name has been updated!").await?;
    } else {
        ctx.say("Submissions are not currently open.").await?;
    }

    Ok(())
}

// Update status command to use Discord timestamps
#[poise::command(slash_command)]
pub async fn status(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let settings = ctx.data().settings.read().await;
    let guild_settings = settings.get_guild_settings(guild_id);

    let status_msg = match &guild_settings.lorax_state {
        LoraxState::Idle => "No active tree naming event.".to_string(),
        LoraxState::Submissions {
            end_time,
            submissions,
            ..
        } => {
            format!(
                "📝 Submissions close {}\n{} submissions so far",
                discord_timestamp(*end_time, TimestampStyle::ShortDateTime),
                submissions.len()
            )
        }
        LoraxState::Voting {
            end_time,
            options,
            votes,
            ..
        } => {
            format!(
                "🗳️ Voting closes {}\n{} options available\n{} votes cast",
                discord_timestamp(*end_time, TimestampStyle::ShortDateTime),
                options.len(),
                votes.len()
            )
        }
    };

    ctx.say(status_msg).await?;
    Ok(())
}

/// View current submissions (Admin only during submission phase)
#[poise::command(slash_command)]
pub async fn list(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let settings = ctx.data().settings.read().await;

    let guild = ctx.guild().unwrap().clone();
    let is_admin = guild.member(ctx.http(), ctx.author().id).await?.permissions(ctx)?.manage_guild();

    if let Some(guild) = settings.guilds.get(&guild_id) {
        match &guild.lorax_state {
            LoraxState::Submissions { submissions, end_time, .. } => {
                if !is_admin {
                    ctx.say("Only administrators can view submissions during the submission phase.").await?;
                    return Ok(());
                }

                let submission_list = submissions
                    .iter()
                    .map(|(user_id, name)| format!("• {} (by <@{}>)", name, user_id))
                    .collect::<Vec<_>>()
                    .join("\n");

                ctx.send(CreateReply::default()
                    .embed(CreateEmbed::default()
                        .title("🌱 Current Submissions")
                        .description(if submission_list.is_empty() {
                            "No submissions yet".to_string()
                        } else {
                            submission_list
                        })
                        .footer(CreateEmbedFooter::new(format!(
                            "Submissions close {} • Total: {}",
                            discord_timestamp(*end_time, TimestampStyle::ShortDateTime),
                            submissions.len()
                        )))
                        .color(Color::from_rgb(67, 160, 71)))
                    .ephemeral(true)
                ).await?;
            }
            LoraxState::Voting { options, end_time, .. } => {
                let options_list = options
                    .iter()
                    .map(|name| format!("• {}", name))
                    .collect::<Vec<_>>()
                    .join("\n");

                ctx.send(CreateReply::default()
                    .embed(CreateEmbed::default()
                        .title("🌳 Submitted Tree Names")
                        .description(options_list)
                        .footer(CreateEmbedFooter::new(format!(
                            "Voting closes {} • Total: {}",
                            discord_timestamp(*end_time, TimestampStyle::ShortDateTime),
                            options.len()
                        )))
                        .color(Color::from_rgb(67, 160, 71)))
                ).await?;
            }
            LoraxState::Idle => {
                ctx.say("No active tree naming event.").await?;
            }
        }
    }

    Ok(())
}

/// Setup the Lorax system for this server
#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn setup(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let settings = ctx.data().settings.read().await;
    let guild_settings = settings.get_guild_settings(guild_id);

    let mut setup_status = vec![];
    
    // Check channel setup
    setup_status.push(format!("📢 Announcement Channel: {}",
        if let Some(channel) = guild_settings.lorax_channel {
            format!("Set to <#{}>", channel)
        } else {
            "❌ Not set - Use `/lorax set_channel`".to_string()
        }
    ));

    // Check role setup
    setup_status.push(format!("👥 Ping Role: {}",
        if let Some(role) = guild_settings.lorax_role {
            format!("Set to <@&{}>", role)
        } else {
            "❌ Not set - Use `/lorax set_role`".to_string()
        }
    ));

    // Current state
    setup_status.push(format!("📋 Current State: {}",
        match guild_settings.lorax_state {
            LoraxState::Idle => "Ready for new event",
            LoraxState::Submissions { .. } => "Submission phase active",
            LoraxState::Voting { .. } => "Voting phase active",
        }
    ));

    ctx.send(CreateReply::default()
        .embed(CreateEmbed::default()
            .title("🌳 Lorax System Setup")
            .description(setup_status.join("\n\n"))
            .footer(CreateEmbedFooter::new(
                if guild_settings.lorax_channel.is_some() && guild_settings.lorax_role.is_some() {
                    "✅ System ready - Use /lorax start to begin a new event"
                } else {
                    "⚠️ Setup incomplete - Configure missing options to start events"
                }
            ))
            .color(if guild_settings.lorax_channel.is_some() && guild_settings.lorax_role.is_some() {
                Color::from_rgb(67, 160, 71)
            } else {
                Color::from_rgb(244, 67, 54)
            }))
    ).await?;

    Ok(())
}

pub async fn handle_button(
    ctx: &serenity::Context,
    component: &ComponentInteraction,
    data: Data,
) -> Result<(), Error> {
    // Handle page navigation
    if component.data.custom_id.starts_with("page_") {
        // Handle pagination - implement if needed
        return Ok(());
    }

    // Handle vote selection
    if component.data.custom_id == "vote_select" {
        let choice = match &component.data.kind {
            serenity::ComponentInteractionDataKind::StringSelect { values } => {
                if let Some(value) = values.first() {
                    value.parse::<usize>().map_err(|_| "Invalid vote selection")?
                } else {
                    return Ok(());
                }
            }
            _ => return Ok(()),
        };

        let guild_id = component.guild_id.unwrap();
        let user_id = component.user.id;

        let mut settings = data.settings.write().await;

        // Validate vote and check end time
        if let Some(guild) = settings.guilds.get(&guild_id) {
            if let LoraxState::Voting {
                options, end_time, ..
            } = &guild.lorax_state {
                if Utc::now().timestamp() > *end_time {
                    let builder = serenity::CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::default()
                            .ephemeral(true)
                            .content("Voting period has ended."),
                    );

                    component.create_response(ctx, builder).await?;
                    return Ok(());
                }

                if let Some(selected_tree) = options.get(choice) {
                    // Check if user is voting for their own submission
                    if let LoraxState::Submissions { submissions, .. } = &guild.lorax_state {
                        if let Some(submitter_id) = submissions
                            .iter()
                            .find(|(_, tree)| *tree == selected_tree)
                            .map(|(id, _)| *id)
                        {
                            if submitter_id == user_id {
                                let builder = serenity::CreateInteractionResponse::Message(
                                    CreateInteractionResponseMessage::default()
                                        .ephemeral(true)
                                        .content("You cannot vote for your own submission."),
                                );

                                component.create_response(ctx, builder).await?;
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }

        // Record the vote
        if let Some(guild) = settings.guilds.get_mut(&guild_id) {
            if let LoraxState::Voting { votes, options, .. } = &mut guild.lorax_state {
                let previous_vote = votes.insert(user_id, choice);

                let tree_name = &options[choice];
                let response = if previous_vote.is_some() {
                    format!("Your vote has been updated to {}! 🗳️", tree_name)
                } else {
                    format!("You voted for {}! 🗳️", tree_name)
                };

                let builder = serenity::CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::default()
                        .ephemeral(true)
                        .content(response),
                );

                component.create_response(ctx, builder).await?;
                
                settings.save()?;
            }
        }
    }

    Ok(())
}
