//! Persisted Reflex policy and adapter capabilities.

const NVIDIA_VENDOR_ID: u32 = 0x10de;

/// Requested NVIDIA Reflex policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Mode {
    /// Do not pace with a latency backend.
    #[default]
    Off,
    /// Keep the GPU from running ahead of simulation when supported.
    On,
    /// Enable Reflex's boost clock policy when supported.
    OnBoost,
}

impl Mode {
    /// Stable order used by the persisted Video settings dropdown.
    pub const ALL: [Self; 3] = [Self::Off, Self::On, Self::OnBoost];

    /// Parses command-line and environment spellings.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "on" => Some(Self::On),
            "on-boost" | "on_boost" | "boost" => Some(Self::OnBoost),
            _ => None,
        }
    }

    /// Reads `CINNABAR_REFLEX` without making an unsupported adapter fatal.
    #[must_use]
    pub fn from_environment() -> Option<Self> {
        std::env::var("CINNABAR_REFLEX")
            .ok()
            .and_then(|value| Self::parse(&value))
    }

    #[must_use]
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Stable persisted index used by the settings registry.
    #[must_use]
    pub const fn index(self) -> i32 {
        match self {
            Self::Off => 0,
            Self::On => 1,
            Self::OnBoost => 2,
        }
    }

    /// Converts a persisted settings index, falling back to the safe default.
    #[must_use]
    pub const fn from_index(index: i32) -> Self {
        match index {
            1 => Self::On,
            2 => Self::OnBoost,
            _ => Self::Off,
        }
    }

    /// User-facing label for the Video settings dropdown.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::On => "On",
            Self::OnBoost => "On + Boost",
        }
    }
}

/// The graphics API used by an adapter.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BackendKind {
    #[default]
    Unknown,
    Vulkan,
    Dx12,
    Metal,
    Other,
}

/// Small adapter identity supplied by the renderer's device-selection boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdapterIdentity {
    pub backend: BackendKind,
    pub vendor_id: u32,
    /// True only after `VK_NV_low_latency2` is enabled on the device.
    pub low_latency2_extension: bool,
    /// True only when the renderer can append `VkSwapchainLatencyCreateInfoNV` before creation.
    pub native_swapchain_hook: bool,
}

impl AdapterIdentity {
    /// The conservative identity used before the renderer publishes a native adapter.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            backend: BackendKind::Unknown,
            vendor_id: 0,
            low_latency2_extension: false,
            native_swapchain_hook: false,
        }
    }

    /// Convenience constructor for a fully wired NVIDIA Vulkan backend.
    #[must_use]
    pub const fn nvidia_vulkan(low_latency2_extension: bool, native_swapchain_hook: bool) -> Self {
        Self {
            backend: BackendKind::Vulkan,
            vendor_id: NVIDIA_VENDOR_ID,
            low_latency2_extension,
            native_swapchain_hook,
        }
    }
}

/// Why a requested mode is or is not active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityState {
    Unsupported,
    /// The driver is NVIDIA Vulkan, but the extension was not enabled on the device.
    ExtensionUnavailable,
    /// The extension is enabled, but swapchain creation has no native pNext hook yet.
    NativeSwapchainHookUnavailable,
    Ready,
}

/// Capability report retained with the frame controller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub backend: BackendKind,
    pub vendor_id: u32,
    pub low_latency2_extension: bool,
    pub native_swapchain_hook: bool,
    pub state: CapabilityState,
}

impl Capabilities {
    #[must_use]
    pub fn detect(adapter: AdapterIdentity) -> Self {
        let state =
            if adapter.backend != BackendKind::Vulkan || adapter.vendor_id != NVIDIA_VENDOR_ID {
                CapabilityState::Unsupported
            } else if !adapter.low_latency2_extension {
                CapabilityState::ExtensionUnavailable
            } else if !adapter.native_swapchain_hook {
                CapabilityState::NativeSwapchainHookUnavailable
            } else {
                CapabilityState::Ready
            };
        Self {
            backend: adapter.backend,
            vendor_id: adapter.vendor_id,
            low_latency2_extension: adapter.low_latency2_extension,
            native_swapchain_hook: adapter.native_swapchain_hook,
            state,
        }
    }

    #[must_use]
    pub const fn ready(self) -> bool {
        matches!(self.state, CapabilityState::Ready)
    }

    /// Whether this adapter is the NVIDIA Vulkan class targeted by Reflex.
    ///
    /// The native extension can still be unavailable or disabled by a driver policy; callers
    /// should use [`Self::ready`] when they need to know whether native pacing is active.
    #[must_use]
    pub fn targets_nvidia_vulkan(self) -> bool {
        self.backend == BackendKind::Vulkan && self.vendor_id == NVIDIA_VENDOR_ID
    }
}
