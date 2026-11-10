//! Whether a window's surface blends with the desktop: the capability check
//! behind the choice of the window frame (specs/gui-title-row.md, *Window
//! frame*; issue 158).
//!
//! Where Rick draws the frame itself it asks for a transparent window, and
//! whether it got one is a property of the surface the renderer draws to.
//! egui-wgpu configures the surface of a transparent window with the
//! pre-multiplied alpha mode where the surface offers it, else with the
//! post-multiplied one, and otherwise, with a warning in the log, leaves it
//! opaque; the margin of a shadow frame is then composited as a black band.
//! So the window is transparent exactly when its surface offers one of
//! those two modes for the adapter in use ([`blends`]).
//!
//! eframe keeps its surfaces to itself. The one place it hands one out is
//! the adapter selector of the wgpu setup, which it calls once, with the
//! first window's surface, right before it asks the surface the same
//! question and configures it. [`Probe`] is that selector: it picks the
//! adapter wgpu's own `request_adapter` picks and notes what the surface
//! offers for it. The answer is there before the first frame is drawn.
//!
//! This is a check of what the surface can do and never of the machine:
//! nothing here reads an adapter's name, its vendor or its driver.

use std::sync::{Arc, OnceLock};

use eframe::egui_wgpu::{NativeAdapterSelectorMethod, WgpuConfiguration, WgpuSetup};
use eframe::wgpu;

/// What the first window's surface was seen to offer: written once, while
/// eframe sets the renderer up, and read when the application is created.
#[derive(Debug, Clone, Default)]
pub struct Probe(Arc<OnceLock<bool>>);

impl Probe {
    /// Whether the first window's surface blends the window with the
    /// desktop; `None` when no surface was seen, which the frame takes for
    /// opaque ([`Mode::of`](crate::frame::Mode::of)).
    #[must_use]
    pub fn blends(&self) -> Option<bool> {
        self.0.get().copied()
    }

    /// eframe's default renderer configuration with this probe as its
    /// adapter selector.
    #[must_use]
    pub fn wgpu_options(&self) -> WgpuConfiguration {
        let mut options = WgpuConfiguration::default();
        if let WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup {
            let power = setup.power_preference;
            let seen = Arc::clone(&self.0);
            let selector: NativeAdapterSelectorMethod = Arc::new(move |adapters, surface| {
                let adapter = pick(adapters, surface, power).ok_or_else(|| {
                    format!(
                        "none of the {} graphics adapters can draw to the window",
                        adapters.len()
                    )
                })?;
                if let Some(surface) = surface {
                    let offered = surface.get_capabilities(&adapter).alpha_modes;
                    // Set once: eframe selects the adapter once.
                    let _ = seen.set(blends(&offered));
                }
                Ok(adapter)
            });
            setup.native_adapter_selector = Some(selector);
        }
        options
    }
}

/// Whether a surface that offers these alpha modes blends a transparent
/// window with the desktop: it does with the pre-multiplied or the
/// post-multiplied mode, the two egui-wgpu configures a transparent window
/// with. `Opaque` does not, and neither does `Inherit`, which leaves the
/// blending to a default nobody set.
#[must_use]
pub fn blends(offered: &[wgpu::CompositeAlphaMode]) -> bool {
    offered.iter().any(|mode| {
        matches!(
            mode,
            wgpu::CompositeAlphaMode::PreMultiplied | wgpu::CompositeAlphaMode::PostMultiplied
        )
    })
}

/// The adapter wgpu's own `request_adapter` picks, which eframe calls when
/// no selector is set (wgpu-core 30, `Instance::request_adapter`): of the
/// adapters that can present to the surface, the best for the power
/// preference.
fn pick(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
    power: wgpu::PowerPreference,
) -> Option<wgpu::Adapter> {
    let able = adapters
        .iter()
        .filter(|adapter| surface.is_none_or(|surface| adapter.is_surface_supported(surface)));
    best(able, |adapter| adapter.get_info().device_type, power).cloned()
}

/// The first of the candidates of the best kind for `power`. wgpu sorts the
/// adapters by kind, stably, and takes the first: among equals the one
/// listed first wins, and the backends list theirs in a fixed order (Vulkan
/// before DX12 before OpenGL).
fn best<T>(
    candidates: impl Iterator<Item = T>,
    kind: impl Fn(&T) -> wgpu::DeviceType,
    power: wgpu::PowerPreference,
) -> Option<T> {
    candidates.min_by_key(|candidate| rank(kind(candidate), power))
}

/// wgpu's order of the kinds of adapter for a power preference, the best
/// first; with no preference none is better than another.
fn rank(kind: wgpu::DeviceType, power: wgpu::PowerPreference) -> u8 {
    use wgpu::DeviceType as Kind;
    use wgpu::PowerPreference as Power;
    match (power, kind) {
        (Power::None, _) => 0,
        (Power::LowPower, Kind::IntegratedGpu) | (Power::HighPerformance, Kind::DiscreteGpu) => 1,
        (Power::LowPower, Kind::DiscreteGpu) | (Power::HighPerformance, Kind::IntegratedGpu) => 2,
        (_, Kind::Other) => 3,
        (_, Kind::VirtualGpu) => 4,
        (_, Kind::Cpu) => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::CompositeAlphaMode as Alpha;
    use wgpu::DeviceType as Kind;
    use wgpu::PowerPreference as Power;

    #[test]
    fn a_surface_blends_with_a_multiplied_alpha_mode_and_with_no_other() {
        assert!(blends(&[Alpha::Opaque, Alpha::PreMultiplied]));
        assert!(blends(&[Alpha::PostMultiplied]));
        assert!(!blends(&[]));
        assert!(!blends(&[Alpha::Opaque]));
        assert!(!blends(&[Alpha::Opaque, Alpha::Inherit]));
        assert!(!blends(&[Alpha::Auto, Alpha::Inherit]));
        // What wgpu's DX12 backend offers for a swap chain on a window
        // handle, its default, and for one on a composition visual.
        assert!(!blends(&[Alpha::Opaque]));
        assert!(blends(&[
            Alpha::Auto,
            Alpha::Inherit,
            Alpha::Opaque,
            Alpha::PostMultiplied,
            Alpha::PreMultiplied,
        ]));
    }

    #[test]
    fn nothing_seen_is_not_known() {
        let probe = Probe::default();
        assert_eq!(probe.blends(), None);
        // The configuration carries the selector that would see it.
        let options = probe.wgpu_options();
        let WgpuSetup::CreateNew(setup) = &options.wgpu_setup else {
            panic!("eframe's default setup creates the instance");
        };
        assert!(setup.native_adapter_selector.is_some());
        assert_eq!(probe.blends(), None, "not before a surface exists");
    }

    /// The order of wgpu-core's `request_adapter`.
    #[test]
    fn the_adapter_is_the_first_of_the_best_kind() {
        let listed = [
            ("vulkan software", Kind::Cpu),
            ("vulkan integrated", Kind::IntegratedGpu),
            ("vulkan discrete", Kind::DiscreteGpu),
            ("dx12 discrete", Kind::DiscreteGpu),
            ("dx12 integrated", Kind::IntegratedGpu),
            ("gl", Kind::Other),
        ];
        let pick = |power| best(listed.iter(), |(_, kind)| *kind, power).map(|(name, _)| *name);
        assert_eq!(pick(Power::HighPerformance), Some("vulkan discrete"));
        assert_eq!(pick(Power::LowPower), Some("vulkan integrated"));
        assert_eq!(pick(Power::None), Some("vulkan software"), "the first");
        // A software adapter is the last resort, after a virtual one.
        let virtual_machine = [("dx12 warp", Kind::Cpu), ("dx12 virtual", Kind::VirtualGpu)];
        let pick = best(
            virtual_machine.iter(),
            |(_, kind)| *kind,
            Power::HighPerformance,
        );
        assert_eq!(pick.map(|(name, _)| *name), Some("dx12 virtual"));
        let none: [(&str, Kind); 0] = [];
        assert!(best(none.iter(), |(_, kind)| *kind, Power::HighPerformance).is_none());
        // The whole order, for both preferences.
        let order = |power| {
            let mut kinds = [
                Kind::Cpu,
                Kind::VirtualGpu,
                Kind::Other,
                Kind::IntegratedGpu,
                Kind::DiscreteGpu,
            ];
            kinds.sort_by_key(|kind| rank(*kind, power));
            kinds
        };
        assert_eq!(
            order(Power::HighPerformance),
            [
                Kind::DiscreteGpu,
                Kind::IntegratedGpu,
                Kind::Other,
                Kind::VirtualGpu,
                Kind::Cpu
            ]
        );
        assert_eq!(
            order(Power::LowPower),
            [
                Kind::IntegratedGpu,
                Kind::DiscreteGpu,
                Kind::Other,
                Kind::VirtualGpu,
                Kind::Cpu
            ]
        );
    }
}
