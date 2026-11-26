use sky_command::complete_command_input;
use sky_data::{generated_entities, generated_items};
use sky_entity::{MobKind, PlayerVitals, WorldMob};
use sky_inventory::{ItemStack, PlayerInventory};
use sky_world::ChunkPos;
use sky_world::terrain::TerrainGenerator;

#[test]
fn cross_crate_survival_inventory_mob_and_worldgen_integration() {
    let mut inv = PlayerInventory::new_starter_kit();
    let sword = ItemStack::from_key("minecraft:diamond_sword", 1).expect("diamond sword");
    assert_eq!(inv.insert(sword), ItemStack::EMPTY);
    assert_eq!(
        generated_entities::by_key("zombie").unwrap().id,
        MobKind::Zombie.protocol_entity_type() as u32
    );
    assert!(generated_items::by_key("golden_apple").is_some());

    let gen_engine = TerrainGenerator::new(12345);
    let batch = gen_engine
        .generate_and_encode_batch_parallel(&[ChunkPos { x: 0, z: 0 }, ChunkPos { x: 1, z: 0 }]);
    assert_eq!(batch.len(), 2);

    let mut mob = WorldMob::new(10, MobKind::Pig, [8.0, -59.0, 8.0]);
    let _ = mob.step_ai(5, None, |x, z| gen_engine.surface_height(x, z));

    let mut vitals = PlayerVitals::new_full(-50.0);
    assert_eq!(vitals.update_movement(-58.0, false, true), 0.0);
    assert!(vitals.update_movement(-58.0, true, true) >= 5.0);

    let (_, _, suggestions) = complete_command_input("/sp", false, &[]);
    assert!(suggestions.contains(&"spawnmob".to_owned()));
}
