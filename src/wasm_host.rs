use bytemuck::Pod;
use serde::Serialize;
use serde::de::DeserializeOwned;

use vzglyd_slide::SlideSpec;

const WIRE_VERSION: u8 = 1;

#[derive(Debug)]
pub enum WireError {
    MissingVersion,
    UnsupportedVersion(u8),
    Deserialize(postcard::Error),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::MissingVersion => write!(f, "missing version byte in slide wire format"),
            WireError::UnsupportedVersion(v) => write!(f, "unsupported slide wire version {v}"),
            WireError::Deserialize(e) => write!(f, "failed to decode slide spec: {e}"),
        }
    }
}

impl std::error::Error for WireError {}

/// Deserialize a versioned wire blob produced by a WASM slide.
pub fn deserialize_spec<V>(bytes: &[u8]) -> Result<SlideSpec<V>, WireError>
where
    V: DeserializeOwned + Serialize + Pod,
{
    let (ver, payload) = bytes.split_first().ok_or(WireError::MissingVersion)?;
    if *ver != WIRE_VERSION {
        return Err(WireError::UnsupportedVersion(*ver));
    }
    postcard::from_bytes(payload).map_err(WireError::Deserialize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrain_slide::{self, Vertex};

    #[test]
    fn wire_roundtrip() {
        let spec = terrain_slide::terrain_slide_spec(0.0, 18.0);
        let mut buf = vec![WIRE_VERSION];
        let mut body = postcard::to_stdvec(&spec).expect("serialize");
        buf.append(&mut body);
        let decoded: vzglyd_slide::SlideSpec<Vertex> = deserialize_spec(&buf).expect("deserialize");
        assert_eq!(decoded.name, spec.name);
        assert_eq!(decoded.draws.len(), spec.draws.len());
        assert_eq!(decoded.textures.len(), spec.textures.len());
    }
}
