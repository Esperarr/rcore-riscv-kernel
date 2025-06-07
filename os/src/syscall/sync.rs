use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    
    let id = if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        process_inner.mutex_info.available[id] = Some(1);
        for i in 0..process_inner.tasks.len() {
            if process_inner.tasks[i].is_some() {
                if let Some(alloc) = process_inner.mutex_info.allocation[i].as_mut() {
                    alloc[id] = Some(0);
                }
                if let Some(need) = process_inner.mutex_info.need[i].as_mut() {
                    need[id] = Some(0);
                }
            }
        }
        id
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_info.available.push(Some(1));
        for i in 0..process_inner.tasks.len() {
            if process_inner.tasks[i].is_some() {
                if let Some(alloc) = process_inner.mutex_info.allocation[i].as_mut() {
                    alloc.push(Some(0));
                }
                if let Some(need) = process_inner.mutex_info.need[i].as_mut() {
                    need.push(Some(0));
                }
            }
        }
        process_inner.mutex_list.len() - 1
    };
    id as isize
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
    // 检查 mutex_id 是否有效
    if mutex_id >= process_inner.mutex_list.len() || process_inner.mutex_list[mutex_id].is_none() {
        return -1;
    }
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    // 先假设本次请求 need[tid][sem_id] += 1
    if let Some(need) = process_inner.mutex_info.need.get_mut(tid) {
        if let Some(need_vec) = need {
            if mutex_id >= need_vec.len() {
                need_vec.resize(mutex_id + 1, Some(0));
            }
            need_vec[mutex_id] = Some(need_vec[mutex_id].unwrap_or(0) + 1);
        }
    }
    // 死锁检测，检查是否安全
    let safe = process_inner.check_mutex_safety(tid, mutex_id);
    // 如果不安全，回滚 need
    if !safe {
        // need[tid][mutex_id] -= 1
        if let Some(need) = process_inner.mutex_info.need.get_mut(tid) {
            if let Some(need_vec) = need {
                if mutex_id < need_vec.len() {
                    if let Some(val) = need_vec[mutex_id] {
                        if val > 0 {
                            need_vec[mutex_id] = Some(val - 1);
                        }
                    }
                }
            }
        }
        return -0xDEAD;
    }
    drop(process_inner);
    mutex.lock();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    // 如果安全，更新资源状态
    // need[tid][mutex_id] -= 1
    if let Some(need) = process_inner.mutex_info.need.get_mut(tid) {
        if let Some(need_vec) = need {
            if mutex_id < need_vec.len() {
                if let Some(val) = need_vec[mutex_id] {
                    if val > 0 {
                        need_vec[mutex_id] = Some(val - 1);
                    }
                }
            }
        }
    }
    // alloc[tid][mutex_id] += 1
    if let Some(alloc) = process_inner.mutex_info.allocation.get_mut(tid) {
        if let Some(alloc_vec) = alloc {
            if mutex_id >= alloc_vec.len() {
                alloc_vec.resize(mutex_id + 1, Some(0));
            }
            alloc_vec[mutex_id] = Some(alloc_vec[mutex_id].unwrap_or(0) + 1);
        }
    }
    // avail[mutex_id] -= 1
    if let Some(avail) = process_inner.mutex_info.available.get_mut(mutex_id) {
        *avail = Some(avail.unwrap_or(1).saturating_sub(1));
    }
    0
}

/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
    // 检查 mutex_id 是否有效
    if mutex_id >= process_inner.mutex_list.len() || process_inner.mutex_list[mutex_id].is_none() {
        return -1;
    }
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    // alloc[tid][mutex_id] -= 1
    if let Some(alloc) = process_inner.mutex_info.allocation.get_mut(tid) {
        if let Some(alloc_vec) = alloc {
            if mutex_id < alloc_vec.len() {
                if let Some(val) = alloc_vec[mutex_id] {
                    alloc_vec[mutex_id] = Some(val.saturating_sub(1));
                }
            }
        }
    }
    // avail[mutex_id] += 1
    if let Some(avail) = process_inner.mutex_info.available.get_mut(mutex_id) {
        *avail = Some(avail.unwrap_or(0) + 1);
    }
    drop(process_inner);
    mutex.unlock();
    0
}

/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        process_inner.semaphore_info.available[id] = Some(res_count as u8);
        for i in 0..process_inner.tasks.len() {
            if process_inner.tasks[i].is_some() {
                if let Some(alloc) = process_inner.semaphore_info.allocation[i].as_mut() {
                    while alloc.len() <= id {
                        alloc.push(Some(0));
                    }
                }
                if let Some(need) = process_inner.semaphore_info.need[i].as_mut() {
                    while need.len() <= id {
                        need.push(Some(0));
                    }
                }
            }
        }
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_info.available.push(Some(res_count as u8));
        for i in 0..process_inner.tasks.len() {
            if process_inner.tasks[i].is_some() {
                if let Some(alloc) = process_inner.semaphore_info.allocation[i].as_mut() {
                    alloc.push(Some(0));
                }
                if let Some(need) = process_inner.semaphore_info.need[i].as_mut() {
                    need.push(Some(0));
                }
            }
        }
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
    // 检查 sem_id 是否有效
    if sem_id >= process_inner.semaphore_list.len() || process_inner.semaphore_list[sem_id].is_none() {
        return -1;
    }
    let semaphore = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    // 更新资源状态
    // alloc[tid][sem_id] -= 1
    if let Some(alloc) = process_inner.semaphore_info.allocation.get_mut(tid) {
        if let Some(vec) = alloc {
            match vec[sem_id] {
                Some(val) if val > 0 => vec[sem_id] = Some(val - 1),
                _ => vec[sem_id] = Some(0),
            }
        }
    }
    // avail[sem_id] += 1
    if let Some(avail) = process_inner.semaphore_info.available.get_mut(sem_id) {
        match *avail {
            Some(val) => *avail = Some(val + 1),
            None => *avail = Some(1),
        }
    }
    drop(process_inner);
    semaphore.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
    // 检查 sem_id 是否有效
    if sem_id >= process_inner.semaphore_list.len() || process_inner.semaphore_list[sem_id].is_none() {
        return -1;
    }
    let semaphore = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    // 如果死锁检测未启用，直接尝试获取信号量
    if !process_inner.deadlock_detection_enabled {
        drop(process_inner);
        semaphore.down();
        return 0;
    }
    // 先假设本次请求 need[tid][sem_id] += 1
    if let Some(need) = process_inner.semaphore_info.need[tid].as_mut() {
        if let Some(val) = need[sem_id] {
            need[sem_id] = Some(val + 1);
        } else {
            need[sem_id] = Some(1);
        }
    }
    // 死锁检测，检查是否安全
    let safe = process_inner.check_semaphore_safety(tid, sem_id);
    // 如果不安全，回滚 need
    if !safe {
        // println!("[DEADLOCK] Thread {} detected deadlock for semaphore {}", tid, sem_id);
        // need[tid][sem_id] -= 1
        if let Some(need) = process_inner.semaphore_info.need[tid].as_mut() {
            if let Some(val) = need[sem_id] {
                if val > 0 {
                    need[sem_id] = Some(val - 1);
                }
            }
        }
        return -0xDEAD;
    }
    // 如果安全，更新资源状态
    // alloc[tid][sem_id] += 1
    if let Some(alloc) = process_inner.semaphore_info.allocation[tid].as_mut() {
        if let Some(val) = alloc[sem_id] {
            alloc[sem_id] = Some(val + 1);
        } else {
            alloc[sem_id] = Some(1);
        }
    }
    // avail[sem_id] -= 1
    if let Some(avail) = process_inner.semaphore_info.available.get_mut(sem_id) {
        if let Some(val) = *avail {
            *avail = Some(val - 1);
        }
    }
    drop(process_inner);
    semaphore.down();
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.deadlock_detection_enabled = enabled == 1;
    0
}
