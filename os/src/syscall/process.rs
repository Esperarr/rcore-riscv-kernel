//! Process management syscalls

use crate::task::{
    change_program_brk, 
    exit_current_and_run_next, 
    suspend_current_and_run_next,
    current_user_token,
    mmap,
    munmap,
    TASK_MANAGER,
};
use crate::mm::{PageTable, VirtAddr};
use crate::timer::get_time_us;
use crate::config::PAGE_SIZE;
use alloc::collections::BTreeMap;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time"); 
    let token = current_user_token();
    let page_table = PageTable::from_token(token);
    if let Some(pa) = page_table.check_and_translate_va(VirtAddr::from(ts as usize), true) {
        let time_val = unsafe { &mut *(pa.0 as *mut TimeVal) };
        let us = get_time_us();
        *time_val = TimeVal {
            sec: us / 1_000_000,
            usec: us % 1_000_000,
        };
        0
    } else {
        -1
    }
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    let token = current_user_token();
    let page_table = PageTable::from_token(token);
    let mut inner = TASK_MANAGER.inner.exclusive_access();
    let current_task_id = inner.current_task;
    let result = match trace_request {
        0 => {
            // Read operation, check read permission
            if let Some(pa) = page_table.check_and_translate_va(VirtAddr::from(id), false) {
                unsafe { *(pa.0 as *const u8) as isize }
            } else {
                -1
            }
        }
        1 => {
            // Write operation, check write permission
            if let Some(pa) = page_table.check_and_translate_va(VirtAddr::from(id), true) {
                unsafe { *(pa.0 as *mut u8) = data as u8; }
                0
            } else {
                -1
            }
        }
        2 => {
            // Return syscall count for current task
            let task_syscall_cnt = inner.task_sys_call_cnt.entry(current_task_id).or_insert_with(BTreeMap::new);
            let count = task_syscall_cnt.get(&id).copied().unwrap_or(0);
            count as isize
        }
        _ => -1,
    };
    drop(inner);
    result
}

// YOUR JOB: Implement mmap.
/// mmap is used to allocate memory for a process.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap NOT IMPLEMENTED YET!");
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    else{
        mmap(start, len, prot)
    }
}

// YOUR JOB: Implement munmap.
/// munmap is used to free memory for a process.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap NOT IMPLEMENTED YET!");
    if start % PAGE_SIZE != 0 {
        return -1;
    } else{
        munmap(start, len)
    }
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
