//! The scene file written by `build.py` (`data/scene.bin`).
//!
//! Little-endian layout:
//!
//! ```text
//! "PNSC"  version  notes  links  tags          4 bytes + 4 x u32
//! centroid x y z   radius  floor               5 x f32
//! default colour                               4 x u8 (r g b a)
//! tag colours                                  tags x 4 x u8
//! positions                                    notes x 3 x f32
//! first tag of each note (0xFFFF for none)     notes x u32
//! links: from, to, kind                        links x 3 x u32
//! ```

use sib::render::glam::Vec3;

const MAGIC: &[u8; 4] = b"PNSC";
const VERSION: u32 = 1;
const NO_TAG: u32 = 0xFFFF;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub from: u32,
    pub to: u32,
    /// True when one note links to the other in its text, false when the two
    /// only share a tag.
    pub explicit: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SceneData {
    pub centroid: Vec3,
    pub radius: f32,
    pub floor: f32,
    pub positions: Vec<Vec3>,
    pub colors: Vec<[f32; 3]>,
    pub links: Vec<Link>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| "the scene file is shorter than its header says".to_owned())?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn vec3(&mut self) -> Result<Vec3, String> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }

    fn color(&mut self) -> Result<[f32; 3], String> {
        let bytes = self.take(4)?;
        Ok([
            f32::from(bytes[0]) / 255.0,
            f32::from(bytes[1]) / 255.0,
            f32::from(bytes[2]) / 255.0,
        ])
    }
}

impl SceneData {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = Reader { bytes, at: 0 };
        if reader.take(4)? != MAGIC {
            return Err("this is not a scene file".to_owned());
        }
        let version = reader.u32()?;
        if version != VERSION {
            return Err(format!("scene file version {version} is not supported"));
        }
        let note_count = reader.u32()? as usize;
        let link_count = reader.u32()? as usize;
        let tag_count = reader.u32()? as usize;

        let centroid = reader.vec3()?;
        let radius = reader.f32()?;
        let floor = reader.f32()?;

        let default_color = reader.color()?;
        let mut palette = Vec::with_capacity(tag_count);
        for _ in 0..tag_count {
            palette.push(reader.color()?);
        }

        let mut positions = Vec::with_capacity(note_count);
        for _ in 0..note_count {
            positions.push(reader.vec3()?);
        }

        let mut colors = Vec::with_capacity(note_count);
        for _ in 0..note_count {
            let tag = reader.u32()?;
            colors.push(if tag == NO_TAG {
                default_color
            } else {
                *palette
                    .get(tag as usize)
                    .ok_or_else(|| format!("a note uses tag {tag}, which is not in the palette"))?
            });
        }

        let mut links = Vec::with_capacity(link_count);
        for _ in 0..link_count {
            let from = reader.u32()?;
            let to = reader.u32()?;
            let kind = reader.u32()?;
            if from as usize >= note_count || to as usize >= note_count {
                return Err(format!("a link joins notes {from} and {to}, but there are {note_count} notes"));
            }
            links.push(Link {
                from,
                to,
                explicit: kind == 1,
            });
        }

        Ok(Self {
            centroid,
            radius,
            floor,
            positions,
            colors,
            links,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        for value in [VERSION, 2, 1, 1] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [1.0_f32, 2.0, 3.0, 9.0, -4.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 255, 0, 255]);
        bytes.extend_from_slice(&[255, 0, 0, 255]);
        for value in [0.0_f32, 1.0, 2.0, 3.0, 4.0, 5.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [0_u32, NO_TAG] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [0_u32, 1, 1] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn reads_a_scene() {
        let scene = SceneData::parse(&sample()).expect("the sample parses");
        assert_eq!(scene.centroid, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(scene.radius, 9.0);
        assert_eq!(scene.floor, -4.0);
        assert_eq!(scene.positions, vec![Vec3::new(0.0, 1.0, 2.0), Vec3::new(3.0, 4.0, 5.0)]);
        assert_eq!(scene.colors, vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        assert_eq!(
            scene.links,
            vec![Link {
                from: 0,
                to: 1,
                explicit: true
            }]
        );
    }

    #[test]
    fn rejects_a_truncated_file() {
        let bytes = sample();
        assert!(SceneData::parse(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn rejects_other_files() {
        assert!(SceneData::parse(b"not a scene").is_err());
    }

    #[test]
    fn rejects_a_link_to_a_missing_note() {
        let mut bytes = sample();
        let at = bytes.len() - 8;
        bytes[at..at + 4].copy_from_slice(&7_u32.to_le_bytes());
        assert!(SceneData::parse(&bytes).is_err());
    }
}
