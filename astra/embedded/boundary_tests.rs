use super::*;

#[test]
fn unsupported_extensions_leave_registration_and_errors_untouched() {
    let initialized = INITIALIZED.load(Ordering::Acquire);
    let active = ACTIVE.load(Ordering::Acquire);
    let mut api = std::ptr::null();
    assert_eq!(unsafe { query(abi::ABI_VERSION, &mut api) }, abi::OK);
    assert!(std::ptr::eq(api, &API));
    for (name, version) in [
        (abi::MODULE_OWNER_EXTENSION, abi::MODULE_OWNER_VERSION),
        (b"unknown.extension".as_slice(), 1),
        (abi::MODULE_OWNER_EXTENSION, u32::MAX),
    ] {
        let mut extension = api.cast();
        assert_eq!(
            unsafe { API.extension.unwrap()(abi::Bytes::new(name), version, &mut extension) },
            abi::UNSUPPORTED,
        );
        assert!(extension.is_null());
    }
    assert_eq!(
        unsafe { API.extension.unwrap()(abi::Bytes::EMPTY, 1, std::ptr::null_mut()) },
        abi::INVALID_ARGUMENT,
    );
    assert_eq!(unsafe { query(0, &mut api) }, abi::UNSUPPORTED);
    assert!(api.is_null());
    assert_eq!(unsafe { query(abi::ABI_VERSION, &mut api) }, abi::OK);
    assert!(std::ptr::eq(api, &API));
    assert_eq!(INITIALIZED.load(Ordering::Acquire), initialized);
    assert_eq!(ACTIVE.load(Ordering::Acquire), active);

    let mut error = abi::Error::default();
    error.set("previous.error", "previous operation failed");
    assert_eq!(boundary(&mut error, || Ok(())), abi::OK);
    assert_eq!(error.code_len, 0);
    assert_eq!(error.message_len, 0);
}

#[test]
fn unsupported_boundary_error_does_not_poison_the_next_call() {
    let mut error = abi::Error::default();
    assert_eq!(
        boundary(&mut error, || Err(abi::UNSUPPORTED.into())),
        abi::UNSUPPORTED,
    );
    assert_ne!(error.code_len, 0);
    assert_eq!(boundary(&mut error, || Ok(())), abi::OK);
    assert_eq!(error.code_len, 0);
    assert_eq!(error.message_len, 0);
    assert_eq!(boundary(std::ptr::null_mut(), || Ok(())), abi::OK);
}

#[test]
fn native_error_preserves_its_cause_chain() {
    let error = unsafe { siglus_scene_vm::host::cstr_required(std::ptr::null(), "device") }
        .expect_err("null native input must fail")
        .context("request_device");
    let failure = Failure::native(error);
    assert_eq!(failure.status, abi::CORE_ERROR);
    assert_eq!(failure.code, "ASTRA_EMU_SIGLUS_ENGINE");
    assert_eq!(failure.message, "request_device: device is null");
}
