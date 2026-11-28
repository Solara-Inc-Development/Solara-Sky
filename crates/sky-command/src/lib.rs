//! Brigadier-style command catalog and `Tab` auto-completion engine for Solara-Sky.

#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub usage: &'static str,
    pub requires_op: bool,
    pub arg_hints: &'static [&'static str],
    pub completes_players: bool,
}

pub static BUILTIN_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        aliases: &["?"],
        usage: "/help",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "list",
        aliases: &[],
        usage: "/list",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "tps",
        aliases: &["mspt"],
        usage: "/tps",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "plugins",
        aliases: &["pl"],
        usage: "/plugins",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "seed",
        aliases: &[],
        usage: "/seed",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "gamemode",
        aliases: &["gm"],
        usage: "/gamemode <survival|creative|adventure|spectator>",
        requires_op: false,
        arg_hints: &["survival", "creative", "adventure", "spectator"],
        completes_players: false,
    },
    CommandSpec {
        name: "tp",
        aliases: &[],
        usage: "/tp <x> <y> <z>",
        requires_op: false,
        arg_hints: &[],
        completes_players: true,
    },
    CommandSpec {
        name: "setblock",
        aliases: &[],
        usage: "/setblock <x> <y> <z> <block>",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "heal",
        aliases: &[],
        usage: "/heal",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "feed",
        aliases: &[],
        usage: "/feed",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "kill",
        aliases: &[],
        usage: "/kill",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "spawnmob",
        aliases: &["summon"],
        usage: "/spawnmob <pig|cow|sheep|chicken|zombie|skeleton|spider|creeper>",
        requires_op: false,
        arg_hints: &[
            "pig", "cow", "sheep", "chicken", "zombie", "skeleton", "spider", "creeper",
        ],
        completes_players: false,
    },
    CommandSpec {
        name: "give",
        aliases: &[],
        usage: "/give <item> [count]",
        requires_op: false,
        arg_hints: &[
            "stone",
            "cobblestone",
            "oak_planks",
            "oak_log",
            "glass",
            "glowstone",
            "diamond_sword",
        ],
        completes_players: false,
    },
    CommandSpec {
        name: "clear",
        aliases: &[],
        usage: "/clear",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "craft",
        aliases: &[],
        usage: "/craft <recipe>",
        requires_op: false,
        arg_hints: &[
            "oak_planks",
            "stick",
            "crafting_table",
            "wooden_pickaxe",
            "wooden_sword",
            "stone_pickaxe",
            "stone_sword",
            "iron_ingot",
            "iron_pickaxe",
            "iron_sword",
            "diamond_pickaxe",
            "diamond_sword",
            "torch",
            "furnace",
            "chest",
            "cooked_beef",
            "bow",
            "arrow",
            "ladder",
            "oak_door",
            "iron_helmet",
            "iron_chestplate",
            "iron_leggings",
            "iron_boots",
            "diamond_chestplate",
            "gold_ingot",
            "golden_apple",
            "shield",
            "red_bed",
            "diamond_helmet",
        ],
        completes_players: false,
    },
    CommandSpec {
        name: "recipes",
        aliases: &[],
        usage: "/recipes",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "inv",
        aliases: &["inventory"],
        usage: "/inv",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "time",
        aliases: &[],
        usage: "/time <day|noon|sunset|night|ticks>",
        requires_op: false,
        arg_hints: &["day", "noon", "sunset", "night"],
        completes_players: false,
    },
    CommandSpec {
        name: "weather",
        aliases: &[],
        usage: "/weather <clear|rain|thunder>",
        requires_op: false,
        arg_hints: &["clear", "rain", "thunder"],
        completes_players: false,
    },
    CommandSpec {
        name: "xp",
        aliases: &["experience"],
        usage: "/xp <points>",
        requires_op: false,
        arg_hints: &["10", "50", "100"],
        completes_players: false,
    },
    CommandSpec {
        name: "effect",
        aliases: &[],
        usage: "/effect <speed|strength|regeneration|resistance|night_vision|clear> [seconds]",
        requires_op: false,
        arg_hints: &[
            "speed",
            "strength",
            "regeneration",
            "resistance",
            "night_vision",
            "clear",
        ],
        completes_players: false,
    },
    CommandSpec {
        name: "rules",
        aliases: &[],
        usage: "/rules",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "guide",
        aliases: &[],
        usage: "/guide",
        requires_op: false,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "save",
        aliases: &[],
        usage: "/save",
        requires_op: true,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "op",
        aliases: &[],
        usage: "/op <player>",
        requires_op: true,
        arg_hints: &[],
        completes_players: true,
    },
    CommandSpec {
        name: "deop",
        aliases: &[],
        usage: "/deop <player>",
        requires_op: true,
        arg_hints: &[],
        completes_players: true,
    },
    CommandSpec {
        name: "kick",
        aliases: &[],
        usage: "/kick <player> [reason]",
        requires_op: true,
        arg_hints: &[],
        completes_players: true,
    },
    CommandSpec {
        name: "ban",
        aliases: &[],
        usage: "/ban <player> [reason]",
        requires_op: true,
        arg_hints: &[],
        completes_players: true,
    },
    CommandSpec {
        name: "pardon",
        aliases: &[],
        usage: "/pardon <player>",
        requires_op: true,
        arg_hints: &[],
        completes_players: false,
    },
    CommandSpec {
        name: "whitelist",
        aliases: &[],
        usage: "/whitelist <add|remove> <player>",
        requires_op: true,
        arg_hints: &["add", "remove"],
        completes_players: false,
    },
];

/// Computes `(replace_start, replace_len, suggestions)` for MC `26.3` `ClientboundCommandSuggestionsPacket`.
pub fn complete_command_input(
    raw_input: &str,
    is_op: bool,
    online_players: &[String],
) -> (usize, usize, Vec<String>) {
    let has_slash = raw_input.starts_with('/');
    let body = raw_input.strip_prefix('/').unwrap_or(raw_input);
    let base_offset = usize::from(has_slash);

    if let Some((cmd_part, rest)) = body.split_once(' ') {
        let lower_cmd = cmd_part.to_ascii_lowercase();
        let arg_start = base_offset + cmd_part.len() + 1;
        // Complete only the first argument token if no second space yet
        if !rest.contains(' ')
            && let Some(spec) = BUILTIN_COMMANDS
                .iter()
                .find(|s| s.name == lower_cmd || s.aliases.contains(&lower_cmd.as_str()))
        {
            if spec.requires_op && !is_op {
                return (arg_start, rest.len(), Vec::new());
            }
            let prefix = rest.to_ascii_lowercase();
            let mut matches: Vec<String> = spec
                .arg_hints
                .iter()
                .filter(|h| h.starts_with(&prefix))
                .map(|&s| s.to_owned())
                .collect();
            if spec.completes_players {
                for p in online_players {
                    if p.to_ascii_lowercase().starts_with(&prefix) {
                        matches.push(p.clone());
                    }
                }
            }
            matches.sort();
            return (arg_start, rest.len(), matches);
        }
        return (raw_input.len(), 0, Vec::new());
    }

    let prefix = body.to_ascii_lowercase();
    let mut out = Vec::new();
    for spec in BUILTIN_COMMANDS {
        if spec.requires_op && !is_op {
            continue;
        }
        if spec.name.starts_with(&prefix) {
            out.push(spec.name.to_owned());
        }
    }
    out.sort();
    (base_offset, body.len(), out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_completer_suggests_root_commands_and_subarguments() {
        let (start, len, list) = complete_command_input("/ga", false, &[]);
        assert_eq!((start, len), (1, 2));
        assert_eq!(list, vec!["gamemode"]);

        let (s2, l2, sub) = complete_command_input("/spawnmob z", false, &[]);
        assert_eq!((s2, l2), (10, 1));
        assert_eq!(sub, vec!["zombie"]);

        let (_, _, op_targets) =
            complete_command_input("/kick Sol", true, &["SolaraAlpha".into(), "Other".into()]);
        assert_eq!(op_targets, vec!["SolaraAlpha"]);
    }
}
