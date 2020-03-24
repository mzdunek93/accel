//! Low-level API for CUDA [context].
//!
//! [context]: https://docs.nvidia.com/cuda/cuda-driver-api/group__CUDA__CTX.html

use crate::{error::*, ffi_call_unsafe, ffi_new_unsafe};
use cuda::*;
use std::{cell::RefCell, rc::Rc};

pub use cuda::CUctx_flags_enum as ContextFlag;

/// Marker struct for CUDA Driver context
#[derive(Debug)]
pub struct Context {
    ptr: CUcontext,
}

impl Drop for Context {
    fn drop(&mut self) {
        ffi_call_unsafe!(cuCtxDestroy_v2, self.ptr).expect("Context remove failed");
    }
}

unsafe impl Send for Context {}
unsafe impl Sync for Context {}

thread_local! {static CONTEXT_STACK_LOCK: Rc<RefCell<Option<CUcontext>>> = Rc::new(RefCell::new(None)) }
fn get_lock() -> Rc<RefCell<Option<CUcontext>>> {
    CONTEXT_STACK_LOCK.with(|rc| rc.clone())
}

impl Context {
    /// Create on the top of context stack
    pub fn create(device: CUdevice, flag: ContextFlag) -> Result<Self> {
        let ptr = ffi_new_unsafe!(cuCtxCreate_v2, flag as u32, device)?;
        if ptr.is_null() {
            panic!("Cannot crate a new context");
        }
        CONTEXT_STACK_LOCK.with(|rc| *rc.borrow_mut() = Some(ptr));
        Ok(Context { ptr })
    }

    /// Check this context is "current" on this thread
    pub fn assure_current(&self) -> Result<()> {
        let current = ffi_new_unsafe!(cuCtxGetCurrent)?;
        if current != self.ptr {
            Err(AccelError::ContextIsNotCurrent)
        } else {
            Ok(())
        }
    }

    pub fn version(&self) -> Result<u32> {
        let mut version: u32 = 0;
        ffi_call_unsafe!(cuCtxGetApiVersion, self.ptr, &mut version as *mut _)?;
        Ok(version)
    }

    /// Push to the context stack of this thread
    pub fn push(&self) -> Result<()> {
        let lock = get_lock();
        if lock.borrow().is_some() {
            return Err(AccelError::ContextDuplicated);
        }
        ffi_call_unsafe!(cuCtxPushCurrent_v2, self.ptr)?;
        *lock.borrow_mut() = Some(self.ptr);
        Ok(())
    }

    /// Pop from the context stack of this thread
    pub fn pop(&self) -> Result<()> {
        let lock = get_lock();
        if lock.borrow().is_none() {
            panic!("No countext has been set");
        }
        let ptr = ffi_new_unsafe!(cuCtxPopCurrent_v2)?;
        if ptr.is_null() {
            panic!("No current context");
        }
        assert!(ptr == self.ptr, "Pop must return same pointer");
        *lock.borrow_mut() = None;
        Ok(())
    }

    /// Block for a context's tasks to complete.
    pub fn sync(&self) -> Result<()> {
        self.assure_current()?;
        ffi_call_unsafe!(cuCtxSynchronize)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::device::*;
    use crate::error::Result;

    #[test]
    fn create() -> Result<()> {
        let device = Device::nth(0)?;
        let ctx = device.create_context_auto()?;
        dbg!(&ctx);
        Ok(())
    }

    #[test]
    fn push() -> Result<()> {
        let device = Device::nth(0)?;
        let ctx = device.create_context_auto()?;
        assert!(ctx.assure_current().is_ok());
        assert!(ctx.push().is_err());
        Ok(())
    }

    #[test]
    fn pop() -> Result<()> {
        let device = Device::nth(0)?;
        let ctx = device.create_context_auto()?;
        assert!(ctx.assure_current().is_ok());
        ctx.pop()?;
        assert!(ctx.assure_current().is_err());
        Ok(())
    }

    #[test]
    fn push_pop() -> Result<()> {
        let device = Device::nth(0)?;
        let ctx = device.create_context_auto()?;
        assert!(ctx.assure_current().is_ok());
        ctx.pop()?;
        assert!(ctx.assure_current().is_err());
        ctx.push()?;
        assert!(ctx.assure_current().is_ok());
        Ok(())
    }

    #[test]
    fn thread() -> Result<()> {
        let device = Device::nth(0)?;
        let ctx1 = device.create_context_auto()?;
        assert!(ctx1.assure_current().is_ok()); // "current" on this thread
        let th = std::thread::spawn(move || -> Result<()> {
            assert!(ctx1.assure_current().is_err()); // ctx1 is NOT current on this thread
            let ctx2 = device.create_context_auto()?;
            assert!(ctx2.assure_current().is_ok());
            Ok(())
        });
        th.join().unwrap()?;
        Ok(())
    }
}