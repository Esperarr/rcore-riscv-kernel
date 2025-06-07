use crate::{
    mm::kernel_token,
    task::{add_task, current_task, TaskControlBlock},
    trap::{trap_handler, TrapContext},
};
use core::ops::DerefMut;

use alloc::{sync::Arc, vec::Vec};
/// thread create syscall
pub fn sys_thread_create(entry: usize, arg: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_thread_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();
    // create a new thread
    let new_task = Arc::new(TaskControlBlock::new(
        Arc::clone(&process),
        task.inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .ustack_base,
        true,
    ));
    // add new task to scheduler
    add_task(Arc::clone(&new_task));
    let new_task_inner = new_task.inner_exclusive_access();
    let new_task_res = new_task_inner.res.as_ref().unwrap();
    let new_task_tid = new_task_res.tid;
    let mut process_inner = process.inner_exclusive_access();
    // add new thread to current process
    let tasks = &mut process_inner.tasks;
    // 扩容 tasks 并在目标索引处插入 new_task
    tasks.resize(new_task_tid + 1, None);
    tasks[new_task_tid] = Some(Arc::clone(&new_task));
    let process_inner = process_inner.deref_mut(); 
    extend_matrix(&mut process_inner.mutex_info.allocation, new_task_tid + 1);
    extend_matrix(&mut process_inner.mutex_info.need, new_task_tid + 1);
    extend_matrix(&mut process_inner.semaphore_info.allocation, new_task_tid + 1);
    extend_matrix(&mut process_inner.semaphore_info.need, new_task_tid + 1);
    // 初始化新线程的资源状态
    let mutex_count = process_inner.mutex_list.len();
    let sem_count = process_inner.semaphore_list.len();
    resize_entry(&mut process_inner.mutex_info.allocation[new_task_tid], mutex_count);
    resize_entry(&mut process_inner.mutex_info.need[new_task_tid], mutex_count);
    resize_entry(&mut process_inner.semaphore_info.allocation[new_task_tid], sem_count);
    resize_entry(&mut process_inner.semaphore_info.need[new_task_tid], sem_count);
    let new_task_trap_cx = new_task_inner.get_trap_cx();
    *new_task_trap_cx = TrapContext::app_init_context(
        entry,
        new_task_res.ustack_top(),
        kernel_token(),
        new_task.kstack.get_top(),
        trap_handler as usize,
    );
    (*new_task_trap_cx).x[10] = arg;
    new_task_tid as isize
}
/// get current thread id syscall
pub fn sys_gettid() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_gettid",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid as isize
}

/// wait for a thread to exit syscall
///
/// thread does not exist, return -1
/// thread has not exited yet, return -2
/// otherwise, return thread's exit code
pub fn sys_waittid(tid: usize) -> i32 {
    trace!(
        "kernel:pid[{}] tid[{}] sys_waittid",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();
    let task_inner = task.inner_exclusive_access();
    let mut process_inner = process.inner_exclusive_access();
    // a thread cannot wait for itself
    if task_inner.res.as_ref().unwrap().tid == tid {
        return -1;
    }
    let mut exit_code: Option<i32> = None;
    let waited_task = process_inner.tasks[tid].as_ref();
    if let Some(waited_task) = waited_task {
        if let Some(waited_exit_code) = waited_task.inner_exclusive_access().exit_code {
            exit_code = Some(waited_exit_code);
        }
    } else {
        // waited thread does not exist
        return -1;
    }
    if let Some(exit_code) = exit_code {
        // dealloc the exited thread
        process_inner.tasks[tid] = None;
        exit_code
    } else {
        // waited thread has not exited
        -2
    }
}
fn resize_entry<T: Default + Copy>(vec_opt: &mut Option<Vec<Option<T>>>, count: usize) {
    if let Some(vec) = vec_opt.as_mut() {
        vec.resize(count, Some(T::default()));
    }
}
fn extend_matrix<T>(matrix: &mut Vec<Option<Vec<Option<T>>>>, len: usize)
where
    T: Default,
{
    matrix.resize_with(len, || Some(Vec::new()));
}

