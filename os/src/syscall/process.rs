//! Process management syscalls
use crate::{
    task::{exit_current_and_run_next, suspend_current_and_run_next, TASK_MANAGER},
    timer::get_time_us,
};
use alloc::collections::btree_map::BTreeMap;


#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("[kernel] Application exited with code {}", exit_code);
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// get time with second and microsecond
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    unsafe {
        *ts = TimeVal {
            sec: us / 1_000_000,
            usec: us % 1_000_000,
        };
    }
    0
}

pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    let mut inner = TASK_MANAGER.inner.exclusive_access();
    let current_task_id = inner.current_task;
    let result=match trace_request {
        0 => {
            let addr = id as *const u8;
            unsafe{ addr.read_volatile() as isize }
        }
        1=> {
            let addr = id as *mut u8;
            unsafe{ addr.write_volatile(data as u8) }
            0
        }
        2=> {
            let task_syscall_cnt = inner.task_sys_call_cnt.entry(current_task_id).or_insert_with(BTreeMap::new);
            let count = task_syscall_cnt.get(&id).copied().unwrap_or(0);
            count as isize // 返回调用次数
        }
        _ => -1,
    };
    drop(inner);
    result
}
