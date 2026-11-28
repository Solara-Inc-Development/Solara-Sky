use sky_api::{CommandEvent, SkyPlugin};
use sky_block_entity::{COOK_DURATION_TICKS, ChestBlockEntity, FurnaceBlockEntity};
use sky_command::complete_command_input;
use sky_data::{generated_entities, generated_items};
use sky_effects::{PlayerEffects, effect_id_from_name};
use sky_entity::{MobKind, PlayerVitals, WorldMob, block_survival_drop};
use sky_inventory::{ItemStack, PlayerInventory};
use sky_projectile::WorldArrow;
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

    for (key, kind) in [
        ("zombie", MobKind::Zombie),
        ("creeper", MobKind::Creeper),
        ("skeleton", MobKind::Skeleton),
        ("spider", MobKind::Spider),
        ("chicken", MobKind::Chicken),
    ] {
        assert_eq!(
            generated_entities::by_key(key).unwrap().id,
            kind.protocol_entity_type() as u32
        );
    }
    assert!(generated_items::by_key("golden_apple").is_some());
    assert_eq!(
        block_survival_drop("minecraft:coal_ore"),
        Some((generated_items::id::COAL, 1))
    );

    let mut chest = ChestBlockEntity::new();
    assert_eq!(
        chest.deposit(ItemStack::new(generated_items::id::IRON_INGOT, 8)),
        ItemStack::EMPTY
    );
    assert_eq!(
        chest.withdraw_first(),
        Some(ItemStack::new(generated_items::id::IRON_INGOT, 8))
    );

    let mut furnace = FurnaceBlockEntity::new();
    assert_eq!(
        furnace
            .interact(ItemStack::new(generated_items::id::RAW_IRON, 1))
            .0,
        1
    );
    assert_eq!(
        furnace
            .interact(ItemStack::new(generated_items::id::COAL, 1))
            .0,
        1
    );
    for _ in 0..COOK_DURATION_TICKS {
        let _ = furnace.tick_smelt();
    }
    assert_eq!(
        furnace.interact(ItemStack::EMPTY).1,
        Some(ItemStack::new(generated_items::id::IRON_INGOT, 1))
    );

    let gen_engine = TerrainGenerator::new(12345);
    let batch = gen_engine
        .generate_and_encode_batch_parallel(&[ChunkPos { x: 0, z: 0 }, ChunkPos { x: 1, z: 0 }]);
    assert_eq!(batch.len(), 2);

    let mut mob = WorldMob::new(10, MobKind::Creeper, [8.0, -59.0, 8.0]);
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

    let (_, _, suggestions) = complete_command_input("/wea", false, &[]);
    assert!(suggestions.contains(&"weather".to_owned()));

    // Phase 9: Armor equipping, damage mitigation, XP levels, and 3D Arrow projectiles
    let chestplate = ItemStack::new(generated_items::id::DIAMOND_CHESTPLATE, 1);
    assert_eq!(inv.equip_armor(chestplate).unwrap().0, 2);
    assert_eq!(inv.total_armor_points(), 8);

    let mut armored_vitals = PlayerVitals::new_full(-58.0);
    let dealt = armored_vitals.apply_damage_with_armor(10.0, inv.total_armor_points());
    assert!(dealt < 7.0, "8 armor points should mitigate 32% of damage");
    assert!(armored_vitals.add_experience(20));
    assert!(armored_vitals.xp_level >= 1);

    let mut arrow = WorldArrow::from_aim(900, 1, [0.0, -56.38, 0.0], 0.0, 0.0);
    arrow.step_flight();
    assert!(arrow.intersects_target([0.0, -58.0, 1.8], 0.4, 1.8));

    // Phase 10: Status potion effects, Golden Apple / Bed / Shield recipes, and Torch block light (14)
    let mut fx = PlayerEffects::new();
    fx.apply(effect_id_from_name("strength").unwrap(), 0, 200);
    assert!((fx.melee_damage_bonus() - 3.0).abs() < 1e-5);
    assert!(sky_inventory::recipes::find_recipe("golden_apple").is_some());
    assert!(sky_inventory::recipes::find_recipe("shield").is_some());
    assert!(sky_inventory::recipes::find_recipe("red_bed").is_some());
    let torch_st = sky_world::generated_blocks::default_state_by_name("minecraft:torch").unwrap();
    assert_eq!(sky_world::lighting::block_emission(torch_st), 14);
}
