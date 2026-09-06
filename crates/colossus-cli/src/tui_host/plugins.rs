use colossus_contracts::{PluginInventoryEntry, PluginStatus};
use colossus_presentation::{
    PresentationBlock, PresentationDocument, PresentationTable, PresentationTone,
};

pub(super) enum PluginsCommand<'a> {
    List,
    Show(&'a str),
    Manage(colossus_contracts::PluginManagementRequest),
}

pub(super) enum PluginCommand<'a> {
    Skills,
    Active,
    Clear,
    Use(&'a str),
    Remove(&'a str),
    Show(&'a str),
    Resources(&'a str),
    Read { skill: &'a str, path: &'a str },
}

pub(super) fn parse_plugins_command(arguments: &str) -> Result<PluginsCommand<'_>, String> {
    let arguments = arguments.trim();
    if arguments.is_empty() || arguments == "list" {
        return Ok(PluginsCommand::List);
    }
    if let Some(name) = arguments.strip_prefix("show ") {
        let name = name.trim();
        if !name.is_empty() && name.split_whitespace().count() == 1 {
            return Ok(PluginsCommand::Show(name));
        }
    }
    use clap::Parser as _;
    let words = shell_words::split(arguments).map_err(|error| format!("/plugins: {error}"))?;
    let cli = crate::Cli::try_parse_from(
        ["colossus".to_owned(), "plugins".to_owned()]
            .into_iter()
            .chain(words),
    )
    .map_err(|error| error.to_string().replace("colossus plugins", "/plugins"))?;
    let crate::Command::Plugins(command) = cli.command else {
        return Err("expected /plugins operation".into());
    };
    Ok(PluginsCommand::Manage(command.command.request()?))
}

pub(super) fn parse_plugin_command(arguments: &str) -> Result<PluginCommand<'_>, String> {
    let arguments = arguments.trim();
    match arguments {
        "skills" => return Ok(PluginCommand::Skills),
        "active" => return Ok(PluginCommand::Active),
        "clear" => return Ok(PluginCommand::Clear),
        _ => {}
    }
    if let Some(value) = single_argument(arguments, "use ") {
        return Ok(PluginCommand::Use(value));
    }
    if let Some(value) = single_argument(arguments, "remove ") {
        return Ok(PluginCommand::Remove(value));
    }
    if let Some(value) = single_argument(arguments, "show ") {
        return Ok(PluginCommand::Show(value));
    }
    if let Some(value) = single_argument(arguments, "resources ") {
        return Ok(PluginCommand::Resources(value));
    }
    if let Some(value) = arguments.strip_prefix("read ") {
        let (skill, path) = value
            .trim()
            .split_once(char::is_whitespace)
            .ok_or_else(plugin_command_usage)?;
        let path = path.trim();
        if !skill.is_empty() && !path.is_empty() && skill.split_whitespace().count() == 1 {
            return Ok(PluginCommand::Read { skill, path });
        }
    }
    Err(plugin_command_usage())
}

fn single_argument<'a>(arguments: &'a str, prefix: &str) -> Option<&'a str> {
    let value = arguments.strip_prefix(prefix)?.trim();
    (!value.is_empty() && value.split_whitespace().count() == 1).then_some(value)
}

fn plugin_command_usage() -> String {
    "/plugin expects skills, active, clear, use PLUGIN/SKILL, remove PLUGIN/SKILL, show PLUGIN/SKILL, resources PLUGIN/SKILL, or read PLUGIN/SKILL PATH".into()
}

pub(super) fn plugins_document(plugins: &[PluginInventoryEntry]) -> PresentationDocument {
    let mut table = PresentationTable::new(
        [
            "Name",
            "Version",
            "Status",
            "Trust",
            "Skills",
            "MCP servers",
        ],
        "No Agent Plugins are installed in this Colossus home.",
    );
    for plugin in plugins {
        table.push_row([
            plugin.manifest.name.clone(),
            plugin.manifest.version.as_deref().unwrap_or("—").into(),
            plugin_status(plugin.status).into(),
            if plugin.origin == colossus_contracts::PluginOrigin::Bundled {
                "Bundled with Colossus".into()
            } else if plugin.trust.trusted {
                "trusted".into()
            } else {
                "untrusted".into()
            },
            plugin.skills.len().to_string(),
            plugin.mcp_servers.len().to_string(),
        ]);
    }
    titled_table("Plugins", table)
}

pub(super) fn plugin_document(plugin: &PluginInventoryEntry) -> PresentationDocument {
    let details = PresentationBlock::KeyValue(vec![
        ("Name".into(), plugin.manifest.name.clone()),
        (
            "Version".into(),
            plugin.manifest.version.as_deref().unwrap_or("—").into(),
        ),
        ("Digest".into(), plugin.digest.clone()),
        ("Status".into(), plugin_status(plugin.status).into()),
        (
            "Trust".into(),
            if plugin.origin == colossus_contracts::PluginOrigin::Bundled {
                "Bundled with Colossus (not a Cosign signature)".into()
            } else if plugin.trust.trusted {
                "trusted".into()
            } else {
                "untrusted".into()
            },
        ),
        ("Source".into(), plugin.source.clone()),
        (
            "Workspace".into(),
            plugin
                .unavailable_reason
                .clone()
                .unwrap_or_else(|| "available".into()),
        ),
    ]);
    let mut document = plugin_skills_document(std::slice::from_ref(plugin), &[]);
    let skills = document.blocks.pop().unwrap_or(PresentationBlock::Text(
        "No Agent Skills are available.".into(),
    ));
    PresentationDocument::from_block(PresentationBlock::Card {
        title: format!("Plugin · {}", plugin.manifest.name),
        tone: PresentationTone::Neutral,
        body: vec![details, skills],
    })
}

pub(super) fn plugin_skills_document(
    plugins: &[PluginInventoryEntry],
    active: &[String],
) -> PresentationDocument {
    let mut table = PresentationTable::new(
        ["ID", "Active", "Description"],
        "No Agent Skills are available.",
    );
    for skill in plugins.iter().flat_map(|plugin| &plugin.skills) {
        table.push_row([
            skill.id.clone(),
            if active.contains(&skill.id) {
                "yes".into()
            } else {
                "no".into()
            },
            skill.description.clone(),
        ]);
    }
    titled_table("Agent Skills", table)
}

fn titled_table(title: &str, table: PresentationTable) -> PresentationDocument {
    PresentationDocument::from_block(PresentationBlock::Card {
        title: title.into(),
        tone: PresentationTone::Neutral,
        body: vec![PresentationBlock::Table(table)],
    })
}

const fn plugin_status(status: PluginStatus) -> &'static str {
    match status {
        PluginStatus::Disabled => "disabled",
        PluginStatus::Enabled => "enabled",
        PluginStatus::Uninstalled => "uninstalled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{
        AGENT_PLUGIN_SCHEMA_V1, AgentPluginManifest, PluginSkillMetadata, PluginTrustEvidence,
    };

    fn plugin() -> PluginInventoryEntry {
        PluginInventoryEntry {
            icon_data_url: None,
            origin: colossus_contracts::PluginOrigin::Bundled,
            available: true,
            unavailable_reason: None,
            actions: Vec::new(),
            manifest: AgentPluginManifest {
                schema: AGENT_PLUGIN_SCHEMA_V1.into(),
                name: "colossus".into(),
                version: Some("1.0.0".into()),
                description: None,
                author: None,
                homepage: None,
                repository: None,
                license: None,
                keywords: Vec::new(),
                extensions: Default::default(),
            },
            digest: format!("sha256:{}", "a".repeat(64)),
            source: "bundled:colossus".into(),
            status: PluginStatus::Enabled,
            trust: PluginTrustEvidence {
                trusted: true,
                profile: Some("bundled".into()),
                signer: Some("colossus-release".into()),
                method: "bundled".into(),
            },
            skills: vec![PluginSkillMetadata {
                id: "colossus/coding".into(),
                plugin: "colossus".into(),
                name: "coding".into(),
                description: "Implement scoped changes.".into(),
                compatibility: None,
                allowed_tools: None,
            }],
            mcp_servers: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn plugin_commands_cover_every_advertised_tui_operation() {
        assert!(matches!(
            parse_plugins_command("show colossus"),
            Ok(PluginsCommand::Show("colossus"))
        ));
        assert!(matches!(
            parse_plugin_command("skills"),
            Ok(PluginCommand::Skills)
        ));
        assert!(matches!(
            parse_plugin_command("resources colossus/coding"),
            Ok(PluginCommand::Resources("colossus/coding"))
        ));
        assert!(matches!(
            parse_plugin_command("read colossus/coding references/checklist.md"),
            Ok(PluginCommand::Read {
                skill: "colossus/coding",
                path: "references/checklist.md"
            })
        ));
    }

    #[test]
    fn plugin_documents_expose_real_names_and_qualified_skills() {
        let plugin = plugin();
        let document = plugins_document(std::slice::from_ref(&plugin));
        let PresentationBlock::Card { body, .. } = &document.blocks[0] else {
            panic!("expected plugin card");
        };
        let PresentationBlock::Table(table) = &body[0] else {
            panic!("expected plugin table");
        };
        assert_eq!(table.rows[0][0], "colossus");

        let document = plugin_skills_document(&[plugin], &["colossus/coding".into()]);
        let PresentationBlock::Card { body, .. } = &document.blocks[0] else {
            panic!("expected skills card");
        };
        let PresentationBlock::Table(table) = &body[0] else {
            panic!("expected skills table");
        };
        assert_eq!(table.rows[0][0], "colossus/coding");
        assert_eq!(table.rows[0][1], "yes");
    }
}
