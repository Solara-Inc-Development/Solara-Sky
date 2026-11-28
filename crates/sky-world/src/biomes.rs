//! Multi-biome climate classification (temperature + humidity FBM) for Overworld worldgen.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceBiome {
    Plains,
    Forest,
    Desert,
    SnowyTaiga,
    RockyPeaks,
}

impl SurfaceBiome {
    /// Classifies `(temperature, humidity)` in `[-1.0, 1.0]` plus elevation `top_y` into a biome.
    pub fn classify(temperature: f64, humidity: f64, top_y: i32, dist_from_spawn: f64) -> Self {
        // Keep the immediate spawn sanctuary pleasant green Plains
        if dist_from_spawn < 36.0 {
            return Self::Plains;
        }
        if top_y >= -36 {
            return Self::RockyPeaks;
        }
        if temperature > 0.28 && humidity < -0.05 {
            Self::Desert
        } else if temperature < -0.26 {
            Self::SnowyTaiga
        } else if humidity > 0.14 {
            Self::Forest
        } else {
            Self::Plains
        }
    }
}
