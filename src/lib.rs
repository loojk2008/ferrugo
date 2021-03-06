#![feature(box_patterns)]
#![feature(repeat_generic_slice)]
#![feature(core_intrinsics)]

#[macro_use]
pub mod macros;
#[macro_use]
pub mod exec;
pub mod class;
pub mod gc;

extern crate libc;
extern crate llvm_sys as llvm;
extern crate rand;
extern crate rustc_hash;
