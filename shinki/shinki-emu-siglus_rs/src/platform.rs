use super::{Failure, Result, abi};
use siglus_scene_vm::runtime::native_ui::{NativeMessageBoxRequest, NativeUiBackend};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Clone, Copy)]
pub(super) struct Cancellation {
    context: usize,
    callback: unsafe extern "C" fn(*mut std::ffi::c_void, u64) -> u32,
}
impl Cancellation {
    pub fn new(service: abi::CancellationService) -> Result<Self> {
        Ok(Self {
            context: service.context as usize,
            callback: service.is_cancelled.ok_or(abi::INVALID_ARGUMENT)?,
        })
    }
    pub fn check(self, epoch: u64) -> Result<()> {
        match unsafe { (self.callback)(self.context as *mut _, epoch) } {
            0 => Ok(()),
            1 => Err(abi::CANCELLED.into()),
            _ => Err(abi::INVALID_ARGUMENT.into()),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Service {
    context: usize,
    request: Option<
        unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *const abi::WindowRequest,
            *mut abi::WindowResult,
            *mut abi::Error,
        ) -> abi::Status,
    >,
    invalidate: Option<unsafe extern "C" fn(*mut std::ffi::c_void, u64, u64)>,
}
impl Service {
    pub fn new(service: abi::WindowService) -> Result<Self> {
        if service.request.is_some() != service.invalidate.is_some() {
            return Err(abi::INVALID_ARGUMENT.into());
        }
        Ok(Self {
            context: service.context as usize,
            request: service.request,
            invalidate: service.invalidate,
        })
    }
    pub fn cancel(self, epoch: u64, context: u64) {
        if let Some(invalidate) = self.invalidate {
            unsafe { invalidate(self.context as *mut _, epoch, context) };
        }
    }
}
pub(super) struct Platform {
    pub service: Service,
    pub epoch: u64,
    pub context: Arc<AtomicU64>,
    pub cancelled: Arc<AtomicBool>,
    pub host_cancel: Cancellation,
    pub audio_paused: Mutex<Arc<AtomicBool>>,
    pub completion: Mutex<Option<Result<(u64, i64)>>>,
}
impl NativeUiBackend for Platform {
    fn show_system_messagebox(&self, request: NativeMessageBoxRequest) {
        let result = self.request(request);
        *self.completion.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
    }
}
impl Platform {
    fn request(&self, request: NativeMessageBoxRequest) -> Result<(u64, i64)> {
        let call = self.service.request.ok_or_else(|| {
            Failure::new(
                abi::UNSUPPORTED,
                "SHINKI_EMU_SIGLUS_WINDOW_SERVICE",
                "native dialog requires a window host",
            )
        })?;
        if self.cancelled.load(Ordering::Acquire) {
            return Err(abi::CANCELLED.into());
        }
        self.host_cancel.check(self.epoch)?;
        if request.title.len() > abi::MAX_MODAL_TEXT
            || request.message.len() > abi::MAX_MODAL_TEXT
            || request.buttons.is_empty()
            || request.buttons.len() > abi::MAX_MODAL_BUTTONS
            || request
                .buttons
                .iter()
                .any(|b| b.label.len() > abi::MAX_MODAL_BUTTON_LABEL)
        {
            return Err(abi::CAPACITY.into());
        }
        let mut buttons = [abi::WindowButton {
            id: 0,
            reserved: 0,
            label: abi::Bytes::EMPTY,
        }; abi::MAX_MODAL_BUTTONS];
        for (index, button) in request.buttons.iter().enumerate() {
            buttons[index] = abi::WindowButton {
                id: index as u32 + 1,
                reserved: 0,
                label: abi::Bytes::new(button.label.as_bytes()),
            };
        }
        let native = abi::WindowRequest {
            session_epoch: self.epoch,
            context_epoch: self.context.load(Ordering::Acquire),
            request_id: request.request_id,
            kind: abi::WINDOW_MODAL,
            flags: 0,
            text: abi::Bytes::new(request.message.as_bytes()),
            pixels: abi::Bytes::EMPTY,
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            autohide_ms: 0,
            buttons: buttons.as_ptr(),
            button_count: request.buttons.len(),
            caption: abi::Bytes::new(request.title.as_bytes()),
            initial_text: abi::Bytes::EMPTY,
        };
        let mut answer = abi::WindowResult {
            selected_button: 0,
            reserved: 0,
            valid_fields: 0,
            fullscreen: 0,
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            input: abi::TextBuffer {
                data: std::ptr::null_mut(),
                capacity: 0,
                len: 0,
            },
            visible: 0,
            minimized: 0,
        };
        let mut error = abi::Error::default();
        let audio = self.audio_paused.lock().map_err(|_| abi::PANICKED)?;
        let previously_paused = audio.swap(true, Ordering::AcqRel);
        let clock_paused = siglus_scene_vm::platform_time::set_paused(true);
        let status = unsafe {
            call(
                self.service.context as *mut _,
                &native,
                &mut answer,
                &mut error,
            )
        };
        siglus_scene_vm::platform_time::set_paused(clock_paused);
        audio.store(previously_paused, Ordering::Release);
        if self.cancelled.load(Ordering::Acquire) {
            return Err(abi::CANCELLED.into());
        }
        self.host_cancel.check(self.epoch)?;
        if status != abi::OK {
            return Err(Failure::new(
                status,
                "SHINKI_EMU_SIGLUS_WINDOW_SERVICE",
                String::from_utf8_lossy(
                    &error.message[..(error.message_len as usize).min(error.message.len())],
                )
                .into_owned(),
            ));
        }
        if answer.reserved != 0 || answer.valid_fields & abi::WINDOW_RESULT_MODAL == 0 {
            return Err(abi::INVALID_ARGUMENT.into());
        }
        let button = answer
            .selected_button
            .checked_sub(1)
            .and_then(|i| request.buttons.get(i as usize))
            .ok_or(abi::INVALID_ARGUMENT)?;
        Ok((request.request_id, button.value))
    }
}
