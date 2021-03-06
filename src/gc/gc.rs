// TODO: CAUTION: Am I doing wrong thing?

use super::super::class::{class::Class, classfile::constant::Constant, classheap::ClassHeap};
use super::super::exec::{
    frame::{AType, Array, Frame, ObjectBody},
    vm::{RuntimeEnvironment, VM},
};
use rustc_hash::FxHashMap;
use std::mem;

pub type GcType<T> = *mut T;

type GcStateMap = FxHashMap<*mut u64, GcTargetInfo>;

#[derive(Debug, Clone, PartialEq, Copy)]
enum GcState {
    Marked,
    Unmarked,
}

#[derive(Debug, Clone, Copy)]
enum GcTargetType {
    Array,
    Object,
    Class,
    ClassHeap,
    ObjectHeap,
    RuntimeEnvironment,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
struct GcTargetInfo {
    pub ty: GcTargetType,
    pub state: GcState,
}

impl GcTargetInfo {
    pub fn new_unmarked(ty_name: &str) -> Self {
        GcTargetInfo {
            ty: match ty_name {
                s if s.ends_with("Array") => GcTargetType::Array,
                s if s.ends_with("ObjectBody") => GcTargetType::Object,
                s if s.ends_with("Class") => GcTargetType::Class,
                s if s.ends_with("ClassHeap") => GcTargetType::ClassHeap,
                s if s.ends_with("ObjectHeap") => GcTargetType::ObjectHeap,
                s if s.ends_with("RuntimeEnvironment") => GcTargetType::RuntimeEnvironment,
                _ => GcTargetType::Unknown,
            },
            state: GcState::Unmarked,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GC {
    allocated_memory: GcStateMap,
    allocated_memory_size_in_byte: usize,
    gc_disabled: bool,
}

impl GC {
    pub fn new() -> GC {
        GC {
            allocated_memory: GcStateMap::default(),
            allocated_memory_size_in_byte: 0,
            gc_disabled: false,
        }
    }

    pub fn alloc<T>(&mut self, val: T) -> GcType<T> {
        let size = mem::size_of_val(&val);
        let ptr = Box::into_raw(Box::new(val));
        let info = GcTargetInfo::new_unmarked(unsafe { std::intrinsics::type_name::<T>() });
        self.allocated_memory_size_in_byte += size;
        self.allocated_memory.insert(ptr as *mut u64, info);
        ptr
    }

    pub fn mark_and_sweep(&mut self, vm: &VM) {
        if self.gc_disabled {
            return;
        }

        let over10mib_allocated = self.allocated_memory_size_in_byte > 10 * 1024 * 1024;
        if !over10mib_allocated {
            return;
        }

        let mut m = GcStateMap::default();
        self.trace(&vm, &mut m);
        self.free(&m);
    }

    fn trace(&mut self, vm: &VM, m: &mut GcStateMap) {
        trace_ptr(&mut self.allocated_memory, m, vm.runtime_env as *mut u64);
        trace_ptr(&mut self.allocated_memory, m, vm.classheap as *mut u64);
        trace_ptr(&mut self.allocated_memory, m, vm.objectheap as *mut u64);

        // trace frame stack
        for frame in &vm.frame_stack {
            frame.trace(&mut self.allocated_memory, m);
        }

        // trace variable stack
        for val in &vm.stack {
            trace_ptr(&mut self.allocated_memory, m, *val as *mut u64);
        }
    }

    fn free(&mut self, m: &GcStateMap) {
        let mut total_released_size = 0;

        self.allocated_memory.retain(|p, info| {
            let is_marked = m
                .get(p)
                .and_then(|info| Some(info.state == GcState::Marked))
                .unwrap_or(false);
            if !is_marked {
                let released_size = free_ptr(*p, info);
                total_released_size += released_size;
            }
            is_marked
        });

        if self.allocated_memory_size_in_byte as isize - total_released_size as isize >= 0 {
            self.allocated_memory_size_in_byte -= total_released_size;
        }
    }

    pub fn enable(&mut self) {
        self.gc_disabled = false;
    }

    pub fn disable(&mut self) {
        self.gc_disabled = true;
    }
}

impl Frame {
    fn trace(&self, allocated: &mut GcStateMap, m: &mut GcStateMap) {
        if let Some(class) = self.class {
            trace_ptr(allocated, m, class as *mut u64);
        }
    }
}

impl Class {
    fn trace(&self, allocated: &mut GcStateMap, traced: &mut GcStateMap) {
        self.static_variables
            .iter()
            .for_each(|(_, v)| trace_ptr(allocated, traced, *v as *mut u64));
        for constant in &self.classfile.constant_pool {
            match constant {
                Constant::Utf8 { java_string, .. } => {
                    if let Some(java_string) = java_string {
                        trace_ptr(allocated, traced, *java_string as *mut u64);
                    }
                }
                _ => {}
            }
        }
    }
}

fn trace_ptr(allocated: &mut GcStateMap, m: &mut GcStateMap, ptr: *mut u64) {
    if ptr == 0 as *mut u64 {
        return;
    }

    if m.contains_key(&ptr) {
        return;
    }

    let mut info = if let Some(info) = allocated.get(&ptr).map(|x| *x) {
        info
    } else {
        return;
    };

    m.insert(ptr, {
        info.state = GcState::Marked;
        info
    });

    match info.ty {
        GcTargetType::Array => {
            // TODO: FIX
            let ary = unsafe { &*(ptr as *mut Array) };
            match ary.atype {
                AType::Class(_) => {
                    let len = ary.get_length();
                    for i in 0..len {
                        trace_ptr(allocated, m, ary.at::<u64>(i as isize) as *mut u64);
                    }
                }
                _ => {}
            }
        }
        GcTargetType::Object => {
            let obj = unsafe { &*(ptr as *mut ObjectBody) };
            unsafe { &*obj.class }.trace(allocated, m);
            obj.variables
                .iter()
                .for_each(|v| trace_ptr(allocated, m, *v as *mut u64));
        }
        GcTargetType::Class => {
            let class = unsafe { &*(ptr as *mut Class) };
            class.trace(allocated, m);
        }
        GcTargetType::ClassHeap => {
            let classheap = unsafe { &*(ptr as *mut ClassHeap) };
            for (_, class_ptr) in &classheap.class_map {
                trace_ptr(allocated, m, *class_ptr as *mut u64);
            }
        }
        GcTargetType::ObjectHeap => {}
        GcTargetType::RuntimeEnvironment => {
            let renv = unsafe { &*(ptr as *mut RuntimeEnvironment) };
            trace_ptr(allocated, m, renv.classheap as *mut u64);
            trace_ptr(allocated, m, renv.objectheap as *mut u64);
        }
        GcTargetType::Unknown => panic!(),
    };
}

fn free_ptr(ptr: *mut u64, info: &GcTargetInfo) -> usize {
    match info.ty {
        GcTargetType::Array => mem::size_of_val(&*unsafe { Box::from_raw(ptr as *mut Array) }),
        GcTargetType::Object => {
            mem::size_of_val(&*unsafe { Box::from_raw(ptr as *mut ObjectBody) })
        }
        GcTargetType::Class => mem::size_of_val(&*unsafe { Box::from_raw(ptr as *mut Class) }),
        GcTargetType::ClassHeap
        | GcTargetType::ObjectHeap
        | GcTargetType::RuntimeEnvironment
        | GcTargetType::Unknown => 0,
    }
}
