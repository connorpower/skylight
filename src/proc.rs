//! Windows process and COM management.

use ::std::{
    cell::RefCell,
    rc::{Rc, Weak},
};
use ::windows::Win32::System::{
    Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
    Memory::{HeapEnableTerminationOnCorruption, HeapSetInformation},
};

use crate::errors::Context;

/// Enables the terminate-on-corruption feature. If the heap manager detects an
/// error in any heap used by the process, it calls the Windows Error Reporting
/// service and terminates the process. After a process enables this feature, it
/// cannot be disabled.
///
/// Returns `true` if heap protection was successfully enabled and `false` if
/// the OS version could not support the request.
pub fn enable_heap_protection() -> bool {
    unsafe { HeapSetInformation(None, HeapEnableTerminationOnCorruption, None, 0).as_bool() }
}

thread_local! {
    static COM_LIBRARY_HANDLE: RefCell<Weak<ComLibraryHandle>> = RefCell::new(Weak::new());
}

/// A RAII object which, while held, ensures the COM library is loaded and
/// initialized in the current thread.
///
/// Acquiring a [`ComLibraryHandle`] will initialize the COM library for use by
/// the calling thread, set the thread's concurrency model to apartment
/// threading, and create a new apartment for the thread if required. This
/// handle must be acquired for every thread that might use COM objects.
///
/// # Threading Model
///
/// The COM library will be initialized to sue the apartment-threading model.
/// Apartment-threading allowing for multiple threads of execution but
/// serializes all incoming calls by requiring that calls to methods of objects
/// created by a thread always run on the same thread, i.e. the apartment/thread
/// that created them. In addition, calls can arrive only at message-queue
/// boundaries.
///
/// # Usage
///
/// ```rust
/// use ::skylight::proc::ComLibraryHandle;
/// use ::std::thread;
///
/// thread::spawn(move || {
///     let _handle = ComLibraryHandle::acquire();
///
///     // some work here
///
///     // `_handle` is dropped and COM resources are uninitialized.
/// });
/// ```
pub struct ComLibraryHandle(());

impl ComLibraryHandle {
    /// Acquire a ref-counted handle to the COM library for the calling thread.
    ///
    /// This should ideally be called only once on thread creation on dropped on
    /// thread termination, but repeated calls to [`acquire`] will not cause
    /// problems due to the ref-counted return type provided all returned values
    /// are dropped appropriately when the thread exits.
    ///
    /// [`acquire`]: Self::acquire
    pub fn acquire() -> Rc<Self> {
        COM_LIBRARY_HANDLE.with(|cell| {
            let cell_ref = cell.borrow();
            if let Some(h) = Weak::upgrade(&*cell_ref) {
                h
            } else {
                drop(cell_ref);
                ::tracing::debug!("Initializing COM library (apartment-threaded)");
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                    .context("Failed to initialize COM library for thread")
                    .function("CoInitializeEx")
                    .unwrap();
                let handle = Rc::new(Self(()));
                cell.replace(Rc::downgrade(&handle));
                handle
            }
        })
    }
}

impl Drop for ComLibraryHandle {
    fn drop(&mut self) {
        ::tracing::debug!("Uninitializing COM library");
        unsafe {
            CoUninitialize();
        }
    }
}
