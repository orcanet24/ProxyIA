// src/positional.rs
// Posicionamiento 3D estable + Force-Directed Layout.
//
//  X = hash(path_del_directorio)   → agrupa archivos del mismo directorio
//  Y = hash(path_del_archivo)      → identifica cada archivo dentro del dir
//  Z = profundidad normalizada     → capa de abstracción (0.0 raíz, 0.1 c/nivel)
//
// Tras escanear dependencias, ForceDirectedLayout refina X,Y:
//  - Archivos que se importan mutuamente → se acercan en X
//  - Archivos sin relación → se alejan en X

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Vector 3D con posición final (X_logical, Y_logical, Z)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position3D {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Position3D {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&self.x.to_le_bytes());
        bytes.extend_from_slice(&self.y.to_le_bytes());
        bytes.extend_from_slice(&self.z.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 12 {
            return None;
        }
        let x = f32::from_le_bytes(bytes[0..4].try_into().ok()?);
        let y = f32::from_le_bytes(bytes[4..8].try_into().ok()?);
        let z = f32::from_le_bytes(bytes[8..12].try_into().ok()?);
        Some(Self { x, y, z })
    }
}

/// Genera un hash determinístico y uniforme a partir de un string.
/// Siempre devuelve el mismo valor para la misma entrada.
fn stable_hash(input: &str) -> f32 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut h);
    let hash = h.finish();
    // Mapear u64 a [0.0, 1.0) de forma uniforme
    (hash >> 11) as f32 / (1u64 << 53) as f32
}

/// Posicionador estable basado en hash de paths + profundidad.
/// No usa índices de hermanos, no cambia al añadir/eliminar archivos.
pub struct StablePositioner;

impl StablePositioner {
    /// Calcula posición base (antes de force-directed).
    /// - dir_path: path del directorio contenedor
    /// - file_path: path completo del archivo
    /// - depth: profundidad en el árbol (0 = raíz)
    /// - public_func_count: número de funciones públicas (para Z)
    pub fn calculate_base(dir_path: &str, file_path: &str, depth: i32, public_func_count: i32) -> Position3D {
        let x = stable_hash(dir_path);
        // Y combina el hash del directorio con el del nombre del archivo
        // para que archivos en el mismo directorio tengan X cercana pero Y única
        let y = stable_hash(file_path);
        // Z = profundidad + factor de API surface (funciones públicas)
        // El factor se normaliza a ~0.05 por función pública, hasta 0.5 máximo
        let api_factor = (public_func_count as f32 * 0.05).min(0.5);
        let z = (depth as f32) * 0.1 + api_factor;
        Position3D { x, y, z }
    }

    /// Posición para el nodo raíz del proyecto
    pub fn root_position() -> Position3D {
        Position3D { x: 0.5, y: 0.5, z: 0.0 }
    }
}

/// Force-Directed Layout: refina posiciones X,Y basándose en dependencias.
///
/// Algoritmo:
/// 1. Cada nodo empieza en su posición base (hash).
/// 2. Por cada par (source, target) con dependencia:
///    - Atracción: mueve source.x hacia target.x con fuerza dependiente del peso.
///    - Repulsión suave: separa nodos que NO tienen dependencia pero están cerca.
/// 3. Se itera `iterations` veces.
/// 4. Las posiciones finales son determinísticas (mismas dependencias → mismo resultado).
pub struct ForceDirectedLayout {
    pub iterations: usize,
    pub attraction_strength: f32,
    pub repulsion_strength: f32,
    pub max_displacement: f32,
}

impl Default for ForceDirectedLayout {
    fn default() -> Self {
        Self {
            iterations: 40,        // Más iteraciones para mejor convergencia
            attraction_strength: 0.25,  // Atracción más suave para evitar oscilaciones
            repulsion_strength: 0.08,   // Repulsión más fuerte para mejor separación
            max_displacement: 0.15,     // Menor desplazamiento por iteración para estabilidad
        }
    }
}

impl ForceDirectedLayout {
    /// Refina un conjunto de posiciones base usando las dependencias.
    ///
    /// - base_positions: HashMap<node_id, Position3D> con posiciones base (hash)
    /// - dependencies: Vec<(source_id, target_id)>
    ///
    /// Retorna: HashMap<node_id, Position3D> con posiciones refinadas
    pub fn refine(
        &self,
        base_positions: &HashMap<i64, Position3D>,
        dependencies: &[(i64, i64)],
    ) -> HashMap<i64, Position3D> {
        let mut positions: HashMap<i64, Position3D> = base_positions.clone();
        if positions.len() < 2 || dependencies.is_empty() {
            return positions;
        }

        // Construir índice de dependencias por nodo
        let mut deps_per_node: HashMap<i64, Vec<i64>> = HashMap::new();
        for (src, tgt) in dependencies {
            deps_per_node.entry(*src).or_default().push(*tgt);
            deps_per_node.entry(*tgt).or_default().push(*src);
        }

        let node_ids: Vec<i64> = positions.keys().copied().collect();

        for _iter in 0..self.iterations {
            let mut deltas: HashMap<i64, f32> = HashMap::new();

            // Calcular desplazamiento para cada nodo
            for &node_id in &node_ids {
                let pos = positions[&node_id];
                let mut dx = 0.0f32;

                // Atracción hacia nodos con dependencia
                if let Some(neighbors) = deps_per_node.get(&node_id) {
                    for &neighbor in neighbors {
                        if let Some(&neighbor_pos) = positions.get(&neighbor) {
                            let diff = neighbor_pos.x - pos.x;
                            dx += diff * self.attraction_strength;
                        }
                    }
                }

                // Repulsión suave de nodos sin dependencia directa
                // pero cercanos en X (para evitar que todos colapsen al mismo punto)
                for &other_id in &node_ids {
                    if other_id == node_id {
                        continue;
                    }
                    let has_dep = deps_per_node
                        .get(&node_id)
                        .map(|n| n.contains(&other_id))
                        .unwrap_or(false);
                    if !has_dep {
                        if let Some(&other_pos) = positions.get(&other_id) {
                            let diff = pos.x - other_pos.x;
                            let dist = diff.abs();
                            if dist > 0.0 && dist < 0.3 {
                                // Si están cerca pero no tienen dependencia → repulsión
                                let force = self.repulsion_strength / (dist + 0.01);
                                dx += diff.signum() * force.min(self.max_displacement * 0.5);
                            }
                        }
                    }
                }

                deltas.insert(node_id, dx);
            }

            // Aplicar desplazamientos con clamp
            for (&node_id, &dx) in &deltas {
                if let Some(pos) = positions.get_mut(&node_id) {
                    let clamped = dx.clamp(-self.max_displacement, self.max_displacement);
                    pos.x = (pos.x + clamped).clamp(0.0, 1.0);
                }
            }
        }

        positions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stable_hash_deterministic() {
        let a = stable_hash("src/db.rs");
        let b = stable_hash("src/db.rs");
        assert!((a - b).abs() < f32::EPSILON);
    }

    #[test]
    fn test_stable_hash_different_inputs() {
        let a = stable_hash("src/db.rs");
        let b = stable_hash("src/scanner.rs");
        assert!((a - b).abs() > 0.001); // Muy probablemente diferente
    }

    #[test]
    fn test_base_position() {
        let pos = StablePositioner::calculate_base("/project/src", "/project/src/main.rs", 1, 0);
        assert!(pos.x >= 0.0 && pos.x <= 1.0);
        assert!(pos.y >= 0.0 && pos.y <= 1.0);
        assert!((pos.z - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_same_directory_same_x() {
        let a = StablePositioner::calculate_base("/project/src", "/project/src/db.rs", 1, 3);
        let b = StablePositioner::calculate_base("/project/src", "/project/src/scanner.rs", 1, 0);
        // Mismo directorio → mismo X (el hash es del directorio, no del archivo)
        assert!((a.x - b.x).abs() < f32::EPSILON);
        // Pero diferente Y (el hash del nombre de archivo difiere)
        assert!((a.y - b.y).abs() > 0.001);
    }

    #[test]
    fn test_force_directed_basic() {
        let mut base = HashMap::new();
        base.insert(1, Position3D { x: 0.2, y: 0.5, z: 0.1 });
        base.insert(2, Position3D { x: 0.8, y: 0.5, z: 0.1 });
        base.insert(3, Position3D { x: 0.5, y: 0.5, z: 0.2 });

        let deps = vec![(1, 2)]; // 1 y 2 tienen dependencia

        let layout = ForceDirectedLayout::default();
        let result = layout.refine(&base, &deps);

        // 1 y 2 deben estar más cerca después del force-directed
        let dist_before = (base[&1].x - base[&2].x).abs();
        let dist_after = (result[&1].x - result[&2].x).abs();
        assert!(dist_after < dist_before);
    }

    #[test]
    fn test_force_directed_deterministic() {
        let mut base = HashMap::new();
        base.insert(1, Position3D { x: 0.2, y: 0.5, z: 0.0 });
        base.insert(2, Position3D { x: 0.8, y: 0.5, z: 0.1 });
        base.insert(3, Position3D { x: 0.5, y: 0.5, z: 0.2 });

        let deps = vec![(1, 2), (2, 3)];

        let layout = ForceDirectedLayout::default();
        let r1 = layout.refine(&base, &deps);
        let r2 = layout.refine(&base, &deps);

        for (k, v1) in &r1 {
            let v2 = &r2[k];
            assert!((v1.x - v2.x).abs() < 0.0001);
            assert!((v1.y - v2.y).abs() < 0.0001);
            assert!((v1.z - v2.z).abs() < 0.0001);
        }
    }
}
