//! Indexed triangle mesh, STL reader and basic geometry helpers.

use anyhow::{Context, Result, bail};
use glam::Vec3;
use std::collections::HashMap;

#[derive(Clone, Default, Debug)]
pub struct Mesh {
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
}

impl Mesh {
    /// Builds an indexed mesh from a triangle soup. Vertices with identical
    /// coordinates are merged, so shared edges get shared indices.
    pub fn from_soup(soup: &[[Vec3; 3]]) -> Mesh {
        let mut map: HashMap<[u32; 3], u32> = HashMap::with_capacity(soup.len() * 2);
        let mut vertices = Vec::new();
        let mut triangles = Vec::with_capacity(soup.len());
        let mut index = |v: Vec3, vertices: &mut Vec<Vec3>| -> u32 {
            // -0.0 and 0.0 must weld together.
            let key = [
                (v.x + 0.0).to_bits(),
                (v.y + 0.0).to_bits(),
                (v.z + 0.0).to_bits(),
            ];
            *map.entry(key).or_insert_with(|| {
                vertices.push(v);
                (vertices.len() - 1) as u32
            })
        };
        for t in soup {
            let a = index(t[0], &mut vertices);
            let b = index(t[1], &mut vertices);
            let c = index(t[2], &mut vertices);
            if a != b && b != c && a != c {
                triangles.push([a, b, c]);
            }
        }
        Mesh {
            vertices,
            triangles,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    pub fn bounds(&self) -> Option<Bounds> {
        let first = *self.vertices.first()?;
        let mut b = Bounds {
            min: first,
            max: first,
        };
        for v in &self.vertices {
            b.min = b.min.min(*v);
            b.max = b.max.max(*v);
        }
        Some(b)
    }

    pub fn tri(&self, i: usize) -> [Vec3; 3] {
        let t = self.triangles[i];
        [
            self.vertices[t[0] as usize],
            self.vertices[t[1] as usize],
            self.vertices[t[2] as usize],
        ]
    }

    pub fn normal(&self, i: usize) -> Vec3 {
        let [a, b, c] = self.tri(i);
        (b - a).cross(c - a).normalize_or_zero()
    }

    /// Signed volume in mm^3 (positive for a closed, outward-facing mesh).
    pub fn volume(&self) -> f32 {
        (0..self.triangles.len())
            .map(|i| {
                let [a, b, c] = self.tri(i);
                a.dot(b.cross(c)) as f64
            })
            .sum::<f64>() as f32
            / 6.0
    }

    /// Number of edges that do not have exactly two faces. Zero for a closed manifold.
    pub fn open_edges(&self) -> usize {
        let mut count: HashMap<(u32, u32), i32> = HashMap::new();
        for t in &self.triangles {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *count.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        count.values().filter(|&&n| n != 2).count()
    }
}

/// Reads binary or ASCII STL.
pub fn read_stl(data: &[u8]) -> Result<Mesh> {
    if data.len() >= 84 {
        let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
        if data.len() == 84 + n * 50 {
            let mut soup = Vec::with_capacity(n);
            for i in 0..n {
                let rec = &data[84 + i * 50..84 + (i + 1) * 50];
                let f = |o: usize| f32::from_le_bytes(rec[o..o + 4].try_into().unwrap());
                let v = |o: usize| Vec3::new(f(o), f(o + 4), f(o + 8));
                soup.push([v(12), v(24), v(36)]);
            }
            return Ok(Mesh::from_soup(&soup));
        }
    }
    let text = std::str::from_utf8(data).context("STL is neither binary nor ASCII")?;
    if !text.trim_start().starts_with("solid") {
        bail!("not an STL file");
    }
    let mut soup = Vec::new();
    let mut cur: Vec<Vec3> = Vec::with_capacity(3);
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("vertex") => {
                let mut p = [0f32; 3];
                for c in &mut p {
                    *c = it.next().context("short vertex line")?.parse()?;
                }
                cur.push(Vec3::from(p));
            }
            Some("endfacet") => {
                if cur.len() == 3 {
                    soup.push([cur[0], cur[1], cur[2]]);
                }
                cur.clear();
            }
            _ => {}
        }
    }
    Ok(Mesh::from_soup(&soup))
}

#[cfg(test)]
pub fn cube(size: f32) -> Mesh {
    let s = size;
    let v = |x: f32, y: f32, z: f32| Vec3::new(x * s, y * s, z * s);
    let p = [
        v(0., 0., 0.),
        v(1., 0., 0.),
        v(1., 1., 0.),
        v(0., 1., 0.),
        v(0., 0., 1.),
        v(1., 0., 1.),
        v(1., 1., 1.),
        v(0., 1., 1.),
    ];
    let f = [
        [0, 2, 1],
        [0, 3, 2], // bottom
        [4, 5, 6],
        [4, 6, 7], // top
        [0, 1, 5],
        [0, 5, 4], // front
        [1, 2, 6],
        [1, 6, 5], // right
        [2, 3, 7],
        [2, 7, 6], // back
        [3, 0, 4],
        [3, 4, 7], // left
    ];
    let soup: Vec<[Vec3; 3]> = f
        .iter()
        .map(|t: &[usize; 3]| [p[t[0]], p[t[1]], p[t[2]]])
        .collect();
    Mesh::from_soup(&soup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_is_closed() {
        let m = cube(10.0);
        assert_eq!(m.vertices.len(), 8);
        assert_eq!(m.open_edges(), 0);
        assert!((m.volume() - 1000.0).abs() < 1e-3);
    }

    #[test]
    fn stl_roundtrip() {
        let m = cube(5.0);
        let bytes = crate::export::stl_bytes(&m);
        let back = read_stl(&bytes).unwrap();
        assert_eq!(back.triangles.len(), 12);
        assert_eq!(back.vertices.len(), 8);
    }
}
