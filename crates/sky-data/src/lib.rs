//! Clean-room generated Minecraft: Java Edition 26.3 (protocol 777) protocol & registry tables:
//! items (`1,658`), entity types (`161`), sound events (`1,991`), particles (`128`), and mob effects (`40`).

pub mod generated_effects;
pub mod generated_entities;
pub mod generated_items;
pub mod generated_particles;
pub mod generated_sounds;

const _: () = {
    assert!(generated_items::TOTAL_ITEMS >= 1600);
    assert!(generated_sounds::TOTAL_SOUND_EVENTS >= 1900);
    assert!(generated_particles::TOTAL_PARTICLE_TYPES >= 100);
    assert!(generated_effects::TOTAL_MOB_EFFECTS >= 35);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups_resolve_mc_26_3_items_entities_and_sounds() {
        let stone = generated_items::by_key("minecraft:stone").expect("stone item");
        assert_eq!(generated_items::by_id(stone.id), Some(stone));

        let player = generated_entities::by_key("player").expect("player entity");
        assert_eq!(player.id, generated_entities::id::PLAYER);

        let zombie = generated_entities::by_key("minecraft:zombie").expect("zombie");
        assert_eq!(zombie.id, generated_entities::id::ZOMBIE);
        assert!(generated_sounds::by_key("block.stone.break").is_some());
    }
}
