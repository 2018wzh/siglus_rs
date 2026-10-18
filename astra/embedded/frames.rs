use super::{abi, configuration, Failure, Result};
use siglus_scene_vm::render::Renderer;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) struct Published {
    pub pixels: Arc<Vec<u8>>,
    pub identity: abi::FrameIdentity,
    pub info: abi::SurfaceInfo,
    pub timestamp: u64,
}
impl Published {
    pub fn surface(&self, after: abi::FrameIdentity) -> abi::FrameSurface {
        let same = after == self.identity;
        let cpu = if same {
            empty_lease()
        } else {
            let owner = Arc::into_raw(Arc::clone(&self.pixels));
            abi::CpuLease {
                pixels: abi::Bytes::new(self.pixels.as_slice()),
                owner: owner.cast_mut().cast(),
                release: Some(release),
            }
        };
        abi::FrameSurface {
            state: if same {
                abi::FRAME_UNCHANGED
            } else {
                abi::FRAME_READY
            },
            reserved: 0,
            identity: self.identity,
            timestamp_ns: self.timestamp,
            info: self.info,
            cpu,
        }
    }
}
unsafe extern "C" fn release(owner: *mut std::ffi::c_void) {
    // One Arc strong reference per lease, independent of the session/device.
    unsafe {
        drop(Arc::from_raw(owner.cast::<Vec<u8>>()));
    }
}
pub(super) fn empty_lease() -> abi::CpuLease {
    abi::CpuLease {
        pixels: abi::Bytes::EMPTY,
        owner: std::ptr::null_mut(),
        release: None,
    }
}
pub(super) struct Readback {
    slots: [Option<Arc<Vec<u8>>>; 3],
    reserved: Option<usize>,
    deadline: Option<Instant>,
    pub generation: u64,
}
impl Readback {
    pub fn new() -> Self {
        Self {
            slots: Default::default(),
            reserved: None,
            deadline: None,
            generation: 0,
        }
    }
    pub fn cancel(&mut self, renderer: &mut Renderer) {
        renderer.cancel_frame_readback();
        self.reserved = None;
        self.deadline = None;
    }
    pub fn poll(
        &mut self,
        renderer: &mut Renderer,
        epoch: u64,
        logical: (u32, u32),
        timestamp: u64,
    ) -> Result<Option<Published>> {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.cancel(renderer);
            return Err(Failure::new(
                abi::CORE_ERROR,
                "ASTRA_EMU_SIGLUS_READBACK_TIMEOUT",
                "native frame readback exceeded five seconds",
            ));
        }
        if self.reserved.is_none() && renderer.hosted_frame_generation() == self.generation {
            return Ok(None);
        }
        let index = if let Some(index) = self.reserved {
            index
        } else {
            let Some(index) = self
                .slots
                .iter()
                .position(|s| s.as_ref().is_none_or(|p| Arc::strong_count(p) == 1))
            else {
                return Ok(None);
            };
            self.reserved = Some(index);
            self.deadline = Some(Instant::now() + Duration::from_secs(5));
            index
        };
        let slot = self.slots[index].get_or_insert_with(|| Arc::new(Vec::new()));
        let pixels = Arc::get_mut(slot).expect("reserved pixels have no consumer");
        let completed = renderer.poll_frame_rgba(pixels).map_err(Failure::native)?;
        let Some((width, height, generation)) = completed else {
            return Ok(None);
        };
        let info = configuration::surface(width, height, logical)?;
        if pixels.len() != info.cpu_bytes()? {
            return Err(abi::CORE_ERROR.into());
        }
        self.reserved = None;
        self.deadline = None;
        self.generation = generation;
        Ok(Some(Published {
            pixels: Arc::clone(slot),
            identity: abi::FrameIdentity {
                session_epoch: epoch,
                output_epoch: 1,
                generation,
            },
            info,
            timestamp,
        }))
    }
}
