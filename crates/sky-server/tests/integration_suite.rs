use sky_api::{CommandEvent, SkyPlugin};
use sky_command::complete_command_input;
use sky_data::{generated_entities, generated_items};
use sky_entity::{MobKind, PlayerVitals, WorldMob, block_survival_drop};
use sky_inventory::{ItemStack, PlayerInventory};
use sky_scripting::LuaScriptPlugin;
use sky_world::ChunkPos;
use sky_world::terrain::TerrainGenerator;
use uuid::Uuid;

#[test]
fn cross_crate_survival_crafting_ores_lua_and_worldgen_integration() {
    let mut inv = PlayerInventory::new_starter_kit();
    let crafted_table = inv.craft_recipe("crafting_table").expect("craft table");
    assert_eq!(
        crafted_table,
        ItemStack::from_key("minecraft:crafting_table", 1).unwrap()
    );

    let sword = ItemStack::from_key("minecraft:diamond_sword", 1).expect("diamond sword");
    assert_eq!(inv.insert(sword), ItemStack::EMPTY);
    assert!(sword.melee_attack_damage() > 7.0);

    assert_eq!(
        generated_entities::by_key("zombie").unwrap().id,
        MobKind::Zombie.protocol_entity_type() as u32
    );
    assert!(generated_items::by_key("golden_apple").is_some());
    assert_eq!(
        block_survival_drop("minecraft:coal_ore"),
        Some((generated_items::id::COAL, 1))
    );

    let gen_engine = TerrainGenerator::new(12345);
    let batch = gen_engine
        .generate_and_encode_batch_parallel(&[ChunkPos { x: 0, z: 0 }, ChunkPos { x: 1, z: 0 }]);
    assert_eq!(batch.len(), 2);

    let mut mob = WorldMob::new(10, MobKind::Pig, [8.0, -59.0, 8.0]);
    let _ = mob.step_ai(5, None, |x, z| gen_engine.surface_height(x, z));

    let mut vitals = PlayerVitals::new_full(-50.0);
    assert_eq!(vitals.update_movement(-58.0, false, true), 0.0);
    assert!(vitals.update_movement(-58.0, true, true) >= 5.0);

    let lua = LuaScriptPlugin::from_lua_source(
        "welcome",
        r#"
        plugin_id = "welcome"
        plugin_name = "WelcomeLua"
        command("rules", "Play fair, {player}!")
        "#,
    )
    .expect("lua parse");
    let mut cmd_ev = CommandEvent {
        sender_username: "Steve".into(),
        sender_uuid: Uuid::nil(),
        is_op: false,
        command: "rules".into(),
        args: vec![],
        response_lines: vec![],
        handled: false,
    };
    lua.on_command(&mut cmd_ev);
    assert!(cmd_ev.handled);
    assert_eq!(cmd_ev.response_lines, vec!["Play fair, Steve!"]);

    let (_, _, suggestions) = complete_command_input("/cr", false, &[]);
    assert!(suggestions.contains(&"craft".to_owned()));
}
