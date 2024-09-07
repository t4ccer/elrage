#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use core::{ffi::CStr, ptr};
use std::string::FromUtf8Error;

use internal::{emacs_env, emacs_runtime, emacs_value};

pub mod internal {
    #![allow(dead_code)]

    include!(concat!(env!("OUT_DIR"), "/emacs_module.rs"));
}

#[derive(Clone, Copy)]
pub struct EmacsEnv {
    inner: *mut emacs_env,
}

unsafe impl Sync for EmacsEnv {}
unsafe impl Send for EmacsEnv {}

impl EmacsEnv {
    #[inline]
    pub fn from_runtime(ert: *mut emacs_runtime) -> EmacsEnv {
        unsafe { Self::from_env(((*ert).get_environment.unwrap())(ert)) }
    }

    #[inline]
    pub fn from_env(env: *mut emacs_env) -> EmacsEnv {
        Self { inner: env }
    }

    #[inline]
    pub fn intern(&self, atom: &CStr) -> emacs_value {
        unsafe {
            ((*self.inner).intern.unwrap())(self.inner, atom.to_bytes_with_nul().as_ptr().cast())
        }
    }

    #[inline]
    pub fn make_function(
        &self,
        min_arity: usize,
        max_arity: usize,
        func: unsafe extern "C" fn(
            env: *mut emacs_env,
            nargs: isize,
            args: *mut emacs_value,
            data: *mut ::std::os::raw::c_void,
        ) -> emacs_value,
        docstring: &CStr,
    ) -> emacs_value {
        unsafe {
            ((*self.inner).make_function.unwrap())(
                self.inner,
                min_arity as isize,
                max_arity as isize,
                Some(func),
                docstring.as_ptr().cast(),
                ptr::null_mut(),
            )
        }
    }

    #[inline]
    pub fn make_string(&self, string: &[u8]) -> emacs_value {
        unsafe {
            ((*self.inner).make_string.unwrap())(
                self.inner,
                string.as_ptr().cast(),
                string.len() as isize,
            )
        }
    }

    #[inline]
    pub fn fun_call(&self, func: emacs_value, args: &[emacs_value]) -> emacs_value {
        unsafe {
            ((*self.inner).funcall.unwrap())(
                self.inner,
                func,
                args.len() as isize,
                args.as_ptr().cast_mut(),
            )
        }
    }

    #[inline]
    pub fn create_function(
        &self,
        func_name: &CStr,
        min_arity: usize,
        max_arity: usize,
        func: unsafe extern "C" fn(
            env: *mut emacs_env,
            nargs: isize,
            args: *mut emacs_value,
            data: *mut ::std::os::raw::c_void,
        ) -> emacs_value,
        docstring: &CStr,
    ) -> emacs_value {
        let fset = self.intern(c"fset");
        let func_name = self.intern(func_name);
        let func = self.make_function(min_arity, max_arity, func, docstring);
        self.fun_call(fset, &[func_name, func]);
        func_name
    }

    #[inline]
    pub fn provide(&self, atom: &CStr) {
        let provide = self.intern(c"provide");
        let atom = self.intern(atom);
        self.fun_call(provide, &[atom]);
    }

    #[inline]
    pub fn copy_string(&self, source: emacs_value, destination: &mut Vec<u8>) {
        let mut required_size: isize = 0;
        unsafe {
            // Reading to null pointer doesn't actually read anything but sets
            // size parameter to required buffer size
            ((*self.inner).copy_string_contents.unwrap())(
                self.inner,
                source,
                ptr::null_mut(),
                (&mut required_size) as *mut _,
            );

            destination.reserve(required_size as usize);
            let copy_successful = ((*self.inner).copy_string_contents.unwrap())(
                self.inner,
                source,
                destination.as_mut_ptr().cast(),
                (&mut required_size) as *mut _,
            );

            assert!(copy_successful);

            // minus 1 to account for null byte
            destination.set_len(required_size as usize - 1);
        }
    }

    #[inline]
    pub fn copy_string_to_string(&self, source: emacs_value) -> Result<String, FromUtf8Error> {
        let mut buf = Vec::new();
        self.copy_string(source, &mut buf);
        String::from_utf8(buf)
    }

    #[inline]
    pub fn make_integer(&self, source: i64) -> emacs_value {
        unsafe { ((*self.inner).make_integer.unwrap())(self.inner, source) }
    }

    #[inline]
    pub fn extract_integer(&self, source: emacs_value) -> i64 {
        unsafe { ((*self.inner).extract_integer.unwrap())(self.inner, source) }
    }

    #[inline]
    pub fn is_not_nil(&self, value: emacs_value) -> bool {
        unsafe { ((*self.inner).is_not_nil.unwrap())(self.inner, value) }
    }
}
