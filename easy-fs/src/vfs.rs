use core::slice::ChunksExact;
use crate::BLOCK_SZ;
// use log::{info, warn, error};

use super::{
    block_cache_sync_all, get_block_cache, BlockDevice, DirEntry, DiskInode, DiskInodeType,
    EasyFileSystem, DIRENT_SZ,
};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::{Mutex, MutexGuard};
/// Virtual filesystem layer over easy-fs
pub struct Inode {
    block_id: usize,
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
}

impl Inode {
    /// Create a vfs inode
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
        }
    }
    /// Call a function over a disk inode to read it
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }
    /// Call a function over a disk inode to modify it
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }
    /// Find inode under a disk inode by name
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        // assert it is a directory
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        let mut dirent = DirEntry::empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                DIRENT_SZ,
            );
            if dirent.name() == name {
                return Some(dirent.inode_id() as u32);
            }
        }
        None
    }
    /// Find inode under current inode by name
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.find_inode_id(name, disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Self::new(
                    block_id,
                    block_offset,
                    self.fs.clone(),
                    self.block_device.clone(),
                ))
            })
        })
    }
    /// Increase the size of a disk inode
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }
    /// Create inode under current inode by name
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        let op = |root_inode: &DiskInode| {
            // assert it is a directory
            assert!(root_inode.is_dir());
            // has the file been created?
            self.find_inode_id(name, root_inode)
        };
        if self.read_disk_inode(op).is_some() {
            return None;
        }
        // create a new file
        // alloc a inode with an indirect block
        let new_inode_id = fs.alloc_inode();
        // initialize inode
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
            });
        self.modify_disk_inode(|root_inode| {
            // append file in the dirent
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            // increase size
            self.increase_size(new_size as u32, root_inode, &mut fs);
            // write dirent
            let dirent = DirEntry::new(name, new_inode_id);
            root_inode.write_at(
                file_count * DIRENT_SZ,
                dirent.as_bytes(),
                &self.block_device,
            );
        });

        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);
        block_cache_sync_all();
        // return inode
        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
        // release efs lock automatically by compiler
    }
    /// List inodes under current inode
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SZ;
            let mut v: Vec<String> = Vec::new();
            for i in 0..file_count {
                let mut dirent = DirEntry::empty();
                assert_eq!(
                    disk_inode.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                v.push(String::from(dirent.name()));
            }
            v
        })
    }
    /// Read data from current inode
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }
    /// Write data to current inode
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }
    /// Clear the data in current inode
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });
        block_cache_sync_all();
    }
    /// link a file under current inode by name
    pub fn linkat(&self, old_name: &str, new_name: &str) -> isize {
        let mut fs = self.fs.lock();
        // check if the old name exists
        let old_node_id = self.read_disk_inode(|disk_inode| {
            self.find_inode_id(old_name, disk_inode)
        });
        if old_node_id.is_none() {
            return -1;
        }
        let old_node_id = old_node_id.unwrap();
        // check if the new name already exists
        let new_node_id = self.read_disk_inode(|disk_inode| {
            self.find_inode_id(new_name, disk_inode)
        });
        if new_node_id.is_some() {
            return -1;
        }
        // 增加目标 inode 的 nlink 计数
        let (block_id, block_offset) = fs.get_disk_inode_pos(old_node_id);
        get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(block_offset, |disk_inode: &mut DiskInode| {
                disk_inode.nlink += 1;
            });
        let res = self.modify_disk_inode(|root_inode| {
            assert!(root_inode.is_dir());
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            self.increase_size(new_size as u32, root_inode, &mut fs);
            let dirent = DirEntry::new(new_name, old_node_id);
            root_inode.write_at(
                file_count * DIRENT_SZ,
                dirent.as_bytes(),
                &self.block_device,
            );
            0
        });
        block_cache_sync_all();
        res
    }
    // /// Unlink a file under current inode by name
    // pub fn unlinkat(&self, name: &str) -> isize {
    //     let mut fs = self.fs.lock();
    //     // 检查文件是否存在
    //     let inode_id = self.read_disk_inode(|disk_inode| {
    //         self.find_inode_id(name, disk_inode)
    //     });
    //     if inode_id.is_none() {
    //         return -1;
    //     }
    //     let inode_id = inode_id.unwrap();
    //     // 获取目标inode
    //     let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
    //     let target_inode = Self::new(
    //         block_id,
    //         block_offset,
    //         self.fs.clone(),
    //         self.block_device.clone(),
    //     );
    //     // 删除目录项
    //     let res = self.modify_disk_inode(|root_inode| {
    //         // 确保是目录
    //         assert!(root_inode.is_dir());
    //         let file_count = (root_inode.size as usize) / DIRENT_SZ;
    //         let mut dirent = DirEntry::empty();
    //         let mut found = false;
    //         let mut found_idx = None;
    //         // 查找目标文件
    //         for i in 0..file_count {
    //             assert_eq!(
    //                 root_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
    //                 DIRENT_SZ,
    //             );
    //             if dirent.name() == name {
    //                 found = true;
    //                 found_idx = Some(i);
    //                 break;
    //             }
    //         }
    //         if !found || found_idx.is_none() {
    //             return -1;
    //         }
    //         let idx = found_idx.unwrap();
    //         // 检查inode是否被其他目录项引用
    //         let mut referenced = false;
    //         let mut check_dirent = DirEntry::empty();
    //         for i in 0..file_count {
    //             if i == idx {
    //                 continue;
    //             }
    //             assert_eq!(
    //                 root_inode.read_at(DIRENT_SZ * i, check_dirent.as_bytes_mut(), &self.block_device,),
    //                 DIRENT_SZ,
    //             );
    //             if check_dirent.inode_id() as u32 == inode_id {
    //                 referenced = true;
    //                 break;
    //             }
    //         }
    //         // 若被删除的目录项不在最后，则需要将最后一个目录项移动到被删除的目录项的位置
    //         let mut last_dirent = DirEntry::empty();
    //         if idx < file_count - 1 {
    //             assert_eq!(
    //                 root_inode.read_at(DIRENT_SZ * (file_count - 1), last_dirent.as_bytes_mut(), &self.block_device,),
    //                 DIRENT_SZ,
    //             );
    //             root_inode.write_at(DIRENT_SZ * idx, last_dirent.as_bytes(), &self.block_device);
    //         }
    //         // 更新目录大小
    //         root_inode.size -= DIRENT_SZ as u32;
    //         // 若被删除的目录项的inode没有被其他目录项引用，则可以直接删除inode
    //         if !referenced {
    //             target_inode.modify_disk_inode(|disk_inode| {
    //                 let size = disk_inode.size;
    //                 let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
    //                 assert_eq!(data_blocks_dealloc.len(), DiskInode::total_blocks(size) as usize);
    //                 for data_block in data_blocks_dealloc.into_iter() {
    //                     fs.dealloc_data(data_block);
    //                 }
    //             });
    //             // 这里需要在EasyFileSystem中实现dealloc_inode方法
    //             fs.inode_bitmap.dealloc(&self.block_device, inode_id as usize);
    //         }
    //         0
    //     });
    //     block_cache_sync_all();
    //     res
    // }
    /// unlink the link to a inode
    pub fn unlinkat(&self, path: &str) -> isize {
        let mut fs = self.fs.lock();
        // 检查文件是否存在
        let inode_id = self.read_disk_inode(|disk_inode| {
            self.find_inode_id(path, disk_inode)
        });
        if inode_id.is_none() {
            return -1;
        }
        let inode_id = inode_id.unwrap();
        // 获取目标inode
        let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
        let target_inode = Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        );
        // 删除目录项
        let res = self.modify_disk_inode(|root_inode| {
            // 确保是目录
            assert!(root_inode.is_dir());
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let mut dirent = DirEntry::empty();
            let mut found = false;
            let mut found_idx = None;
            // 查找目标文件
            for i in 0..file_count {
                assert_eq!(
                    root_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                if dirent.name() == path {
                    found = true;
                    found_idx = Some(i);
                    break;
                }
            }
            if !found || found_idx.is_none() {
                return -1;
            }
            let idx = found_idx.unwrap();
            // 检查inode是否被其他目录项引用
            let mut referenced = false;
            let mut check_dirent = DirEntry::empty();
            for i in 0..file_count {
                if i == idx {
                    continue;
                }
                assert_eq!(
                    root_inode.read_at(DIRENT_SZ * i, check_dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                if check_dirent.inode_id() as u32 == inode_id {
                    referenced = true;
                    break;
                }
            }
            // 若被删除的目录项不在最后，则需要将最后一个目录项移动到被删除的目录项的位置
            let mut last_dirent = DirEntry::empty();
            if idx < file_count - 1 {
                assert_eq!(
                    root_inode.read_at(DIRENT_SZ * (file_count - 1), last_dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                root_inode.write_at(DIRENT_SZ * idx, last_dirent.as_bytes(), &self.block_device);
            }
            // 更新目录大小
            root_inode.size -= DIRENT_SZ as u32;
            // 减少目标 inode 的 nlink 计数
            get_block_cache(block_id as usize, Arc::clone(&self.block_device))
                .lock()
                .modify(block_offset, |disk_inode: &mut DiskInode| {
                    disk_inode.nlink -= 1;
                });
            // 若被删除的目录项的inode没有被其他目录项引用，则可以直接删除inode
            if !referenced {
                target_inode.modify_disk_inode(|disk_inode| {
                    let size = disk_inode.size;
                    let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
                    assert_eq!(data_blocks_dealloc.len(), DiskInode::total_blocks(size) as usize);
                    for data_block in data_blocks_dealloc.into_iter() {
                        fs.dealloc_data(data_block);
                    }
                });
                // 这里需要在EasyFileSystem中实现dealloc_inode方法
                fs.inode_bitmap.dealloc(&self.block_device, inode_id as usize);
            }
            0
        });
        block_cache_sync_all();
        res
    }
    /// calculate nlink
    pub fn nlink(&self, inode_id: u32) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|root_inode| {
            let mut nlink = 0;
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let mut dirent = DirEntry::empty();
            for i in 0..file_count {
                assert_eq!(
                    root_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                if dirent.inode_id() == inode_id as u32 {
                    nlink += 1;
                }
            }
            nlink
        })
    }
    /// get the id of the inode
    // pub fn get_id(&self) -> usize {
    //     let fs = self.fs.lock();
    //     let inode_size = core::mem::size_of::<DiskInode>();
    //     let inodes_per_block = (BLOCK_SZ / inode_size) as u32;
    //     let block_offset = (self.block_id - fs.inode_area_start_block as usize) as u32;
    //     let ans_id = block_offset * inodes_per_block + (self.block_offset / inode_size);
    //     ans_id as usize
    // }
    pub fn get_id(&self) -> usize {
        let fs = self.fs.lock();
        let inode_size = core::mem::size_of::<DiskInode>();
        let inodes_per_block = BLOCK_SZ / inode_size; // usize
        let block_offset = self.block_id - fs.inode_area_start_block as usize;
        let ans_id = block_offset * inodes_per_block + self.block_offset / inode_size;
        ans_id
    }
    /// 判断是否为目录
    pub fn is_dir(&self) -> bool {
        self.read_disk_inode(|disk_inode| disk_inode.is_dir())
    }
    /// get inode by inode id
    pub fn get_inode_by_id(&self, inode_id: u32) -> Option<Arc<Inode>> {
        let fs = self.fs.lock();
        let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
    }
    /// get nlink of the inode(inode-inode id)
    pub fn get_nlink(&self, inode_id: usize) -> usize {
        // info!("Getting nlink for inode {}", inode_id);
        let fs = self.fs.lock();
        let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id as u32);
        let nlink = get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .read(block_offset, |disk_inode: &DiskInode| disk_inode.nlink as usize);
        // info!("Nlink for inode {}: {}", inode_id, nlink);
        nlink
    }
    /// get the mode of the inode(inode-inode id)
    pub fn get_mode(&self, inode_id: usize) -> DiskInodeType {
        // info!("Getting mode for inode {}", inode_id);
        let fs = self.fs.lock();
        let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id as u32);
        let mode = get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .read(block_offset, |disk_inode: &DiskInode| disk_inode.type_.clone());
        // info!("Mode for inode {}: {:?}", inode_id, mode);
        mode
    }
}
