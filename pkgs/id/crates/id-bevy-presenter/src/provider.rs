//! The presenter's provider face (design §5).
//!
//! The app declares what it can present in the standard v1 device
//! manifest ([`DeviceManifest`], the `device.info` shape every provider
//! class uses): a pixel display for its render surface, keys and a
//! gamepad for input, and audio output. Nothing in the declaration names
//! the engine.
//!
//! Each surface is then offered under the world's ordinary grant model:
//! the surface is usable only where the world's [`CapLedger`] allows and
//! grants the catalog provider that guards its capability class.
//!
//! | Surface              | Catalog provider    | Class            |
//! | -------------------- | ------------------- | ---------------- |
//! | [`Surface::Render`]  | `Screen`            | `device.screen`  |
//! | [`Surface::Input`]   | `Key`               | `device.keyboard`|
//! | [`Surface::AudioOut`]| `speech.synthesize` | `device.speaker` |
//!
//! `speech.synthesize` is the catalog's only `device.speaker` provider,
//! so audio output is granted through it until the catalog carries a
//! plain audio-out provider. There is no Bevy-specific declaration and no
//! Bevy-specific grant path: [`SurfaceGrants::from_ledger`] is the only
//! way a surface is enabled, and it reads nothing but the ledger.

use std::collections::BTreeSet;

use bevy::prelude::{Res, Resource};
use id_caps::world_caps::{CapError, CapLedger, DeviceManifest, validate_grant_names};

/// One surface the presenter can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Surface {
    /// The 2D render surface.
    Render,
    /// Keyboard and gamepad input.
    Input,
    /// Audio output.
    AudioOut,
}

impl Surface {
    /// Every surface, in declaration order.
    pub const ALL: [Self; 3] = [Self::Render, Self::Input, Self::AudioOut];

    /// The catalog provider whose grant enables this surface.
    #[must_use]
    pub const fn provider(self) -> &'static str {
        match self {
            Self::Render => "Screen",
            Self::Input => "Key",
            Self::AudioOut => "speech.synthesize",
        }
    }

    /// The surface's name in reports.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Render => "render",
            Self::Input => "input",
            Self::AudioOut => "audio_out",
        }
    }
}

/// The presenter's affordances as a v1 device manifest.
///
/// `width` and `height` are the render surface in pixels.
#[must_use]
pub fn device_manifest(width: u32, height: u32) -> DeviceManifest {
    DeviceManifest {
        display: serde_json::json!({
            "kind": "pixel",
            "width": width,
            "height": height,
            "colors": 16_777_216,
        }),
        input: serde_json::json!({
            "keys": 4,
            "encoder": false,
            "pointer": false,
            "gamepad": true,
            "text_entry": "none",
        }),
        sensors: serde_json::json!({ "gnss": null, "microphone": false, "camera": false }),
        audio: serde_json::json!({ "out": "pcm", "in": null }),
        network: serde_json::json!({ "transport": "shell-contract" }),
    }
}

/// Which surfaces the world has granted this presenter.
#[derive(Resource, Debug, Clone, PartialEq, Eq, Default)]
pub struct SurfaceGrants {
    granted: BTreeSet<Surface>,
}

impl SurfaceGrants {
    /// The surfaces whose provider `ledger` allows and grants.
    #[must_use]
    pub fn from_ledger(ledger: &CapLedger) -> Self {
        Self {
            granted: Surface::ALL
                .into_iter()
                .filter(|surface| ledger.is_granted(surface.provider()))
                .collect(),
        }
    }

    /// The surfaces a world grants with `granted` provider names under
    /// the default ceilings and no grant-on-use.
    ///
    /// # Errors
    ///
    /// The names are checked as any admin grant is: an unknown provider
    /// name is refused, never silently ignored.
    pub fn from_granted_names(granted: &[String]) -> Result<Self, CapError> {
        validate_grant_names(granted)
            .map_err(|error| CapError::bad_args_with(error.to_string()))?;
        let ledger = CapLedger::new(granted.iter().cloned().collect(), false);
        Ok(Self::from_ledger(&ledger))
    }

    /// Every surface granted, as a world granting all three providers
    /// would.
    #[must_use]
    pub fn all() -> Self {
        Self {
            granted: Surface::ALL.into_iter().collect(),
        }
    }

    /// Whether `surface` may be produced.
    #[must_use]
    pub fn allows(&self, surface: Surface) -> bool {
        self.granted.contains(&surface)
    }

    /// The granted surfaces' labels, in declaration order.
    #[must_use]
    pub fn labels(&self) -> Vec<&'static str> {
        self.granted.iter().map(|surface| surface.label()).collect()
    }
}

fn allows(grants: Option<&SurfaceGrants>, surface: Surface) -> bool {
    grants.is_some_and(|grants| grants.allows(surface))
}

/// Run condition: the render surface is granted.
pub fn render_granted(grants: Option<Res<'_, SurfaceGrants>>) -> bool {
    allows(grants.as_deref(), Surface::Render)
}

/// Run condition: the input surface is granted.
pub fn input_granted(grants: Option<Res<'_, SurfaceGrants>>) -> bool {
    allows(grants.as_deref(), Surface::Input)
}

/// Run condition: the audio output surface is granted.
pub fn audio_granted(grants: Option<Res<'_, SurfaceGrants>>) -> bool {
    allows(grants.as_deref(), Surface::AudioOut)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use id_caps::cap_class::{CLASS_DEVICE_KEYBOARD, CLASS_DEVICE_SCREEN, CLASS_DEVICE_SPEAKER};
    use id_caps::world_caps::PROVIDER_REGISTRY;

    use super::*;

    #[test]
    fn the_manifest_is_a_standard_v1_device_manifest() {
        let manifest = device_manifest(640, 480);
        let wire = manifest.to_json();
        assert_eq!(wire["manifest"]["v"], 1);
        assert_eq!(DeviceManifest::from_json(&wire).unwrap(), manifest);
        assert_eq!(wire["display"]["kind"], "pixel");
        assert_eq!(wire["display"]["width"], 640);
        assert_eq!(wire["audio"]["out"], "pcm");
        assert_eq!(wire["input"]["gamepad"], true);
        // The declaration names no engine.
        assert!(!wire.to_string().to_lowercase().contains("bevy"));
    }

    #[test]
    fn each_surface_is_guarded_by_a_catalog_provider_of_its_device_class() {
        let expected = [
            (Surface::Render, CLASS_DEVICE_SCREEN),
            (Surface::Input, CLASS_DEVICE_KEYBOARD),
            (Surface::AudioOut, CLASS_DEVICE_SPEAKER),
        ];
        for (surface, class) in expected {
            let manifest = PROVIDER_REGISTRY.get(surface.provider()).unwrap();
            assert_eq!(manifest.class, class, "{surface:?}");
        }
    }

    #[test]
    fn grants_come_from_the_ledger_alone() {
        let all = SurfaceGrants::from_granted_names(&[
            "Screen".to_owned(),
            "Key".to_owned(),
            "speech.synthesize".to_owned(),
        ])
        .unwrap();
        assert_eq!(all, SurfaceGrants::all());

        let render_only = SurfaceGrants::from_granted_names(&["Screen".to_owned()]).unwrap();
        assert!(render_only.allows(Surface::Render));
        assert!(!render_only.allows(Surface::Input));
        assert!(!render_only.allows(Surface::AudioOut));

        // The wildcard grants every surface, as it grants every allowed
        // provider that is not a sensor.
        assert_eq!(
            SurfaceGrants::from_granted_names(&["*".to_owned()]).unwrap(),
            SurfaceGrants::all()
        );

        // A ceiling that leaves a provider out disables its surface even
        // where the grant names it.
        let mut ledger = CapLedger::new(["Screen".to_owned(), "Key".to_owned()].into(), false);
        ledger.set_allowed(["Key".to_owned()].into());
        let grants = SurfaceGrants::from_ledger(&ledger);
        assert!(!grants.allows(Surface::Render));
        assert!(grants.allows(Surface::Input));

        assert!(SurfaceGrants::from_granted_names(&["Screne".to_owned()]).is_err());
        assert_eq!(SurfaceGrants::default().labels(), Vec::<&str>::new());
    }
}
