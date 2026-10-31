use std::{fmt, hash::Hash};

#[repr(C)]
#[derive(Debug, Clone)]
/// Simple type that stores the size of the TLL and a pointer to the sentinel node.
/// This wraps a std::map in C++.
pub struct TllContainer<T, U> {
    /// Pointer to the sentinel node.
    pub sentinel: *mut Tll<T, U>,
    /// How many items are in the TLL.
    pub size: usize,
}

/// These are the nodes of a C++ std::map.
/// In other words, this is a red-black tree.
///
/// The sentinel node is a special node that acts as the parent of the root node.
/// It's key and data are unused.
/// It also acts as the null node when traversing the tree.
///
/// For this reason it's assumed that a pointer will never be null, and instead will point to the sentinel node.
/// A null pointer would cause undefined behavior.
///
/// # Safety
/// There are several functions that return references to Tll nodes.
/// All references must outlive the C++ std::map that they came from.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct Tll<T, U> {
    /// The left child node of this node.
    /// If this is the sentinel node, this points to the leftmost node in the tree.
    pub left: *mut Tll<T, U>,
    /// The parent node of this node.
    /// If this is the sentinel node, this points to the root node.
    pub parent: *mut Tll<T, U>,
    /// The right child node of this node.
    /// If this is the sentinel node, this points to the rightmost node in the tree.
    pub right: *mut Tll<T, U>,
    /// Is this node black?
    pub is_black: bool,
    /// Is this node the sentinel?
    /// This means it is either the root parent node or a null node when traversing the tree.
    pub is_sentinel: bool,
    _padding: [u8; 6],
    pub key: T,
    pub data: U,
}

impl<T: Default, U: Default> TllContainer<T, U> {
    pub fn new() -> Self {
        unsafe {
            let sentinel_ptr = libc::malloc(size_of::<Tll<T, U>>()) as *mut Tll<T, U>;

            let sentinel = Tll {
                left: sentinel_ptr,
                parent: sentinel_ptr,
                right: sentinel_ptr,
                is_black: true,
                is_sentinel: true,
                _padding: [0; 6],
                key: T::default(),
                data: U::default(),
            };

            std::ptr::write(sentinel_ptr, sentinel);

            Self {
                sentinel: sentinel_ptr,
                size: 0,
            }
        }
    }
}

impl<T, U> TllContainer<T, U> {
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.size
    }

    /// # Safety
    /// This returns a reference to the sentinel node.
    ///
    /// We assume that the sentinel node is always valid as long as the TllContainer is valid.
    pub fn get_sentinel(&mut self) -> &mut Tll<T, U> {
        unsafe { &mut *self.sentinel }
    }

    #[allow(dead_code)]
    pub fn get_items(&mut self) -> Vec<&mut U> {
        self.get_sentinel().into()
    }

    pub fn get_map(&mut self) -> std::collections::HashMap<&T, &mut U>
    where
        T: Eq + Hash,
    {
        self.get_sentinel().into()
    }

    pub fn insert(&mut self, key: T, value: U)
    where
        T: Ord + fmt::Debug,
    {
        unsafe {
            // Check if the tree is empty, aka there are no nodes other than the sentinel
            let sentinel = self.sentinel;
            if std::ptr::eq((*sentinel).parent, sentinel) {
                // Tree is empty, insert as root
                let new_node_ptr = Tll::new(key, value, sentinel);
                (*new_node_ptr).is_black = true; // Root must be black

                // Update sentinel to point to new root
                (*self.sentinel).parent = new_node_ptr;
                (*self.sentinel).left = new_node_ptr;
                (*self.sentinel).right = new_node_ptr;

                self.size += 1;
                return;
            }

            // Find insertion point with standard BST insertion logic
            let mut current = (*sentinel).parent;
            let mut parent = sentinel;

            while !(*current).is_sentinel {
                parent = current;
                if key < (*current).key {
                    current = (*current).left;
                } else if key > (*current).key {
                    current = (*current).right;
                } else {
                    // Key already exists, update value
                    (*current).data = value;
                    return;
                }
            }

            let new_node = Tll::new(key, value, sentinel);
            (*new_node).parent = parent;

            if (*new_node).key < (*parent).key {
                (*parent).left = new_node;
            } else {
                (*parent).right = new_node;
            }

            self.size += 1;

            self.insert_fixup(new_node);

            self.update_sentinel_min_max();
        }
    }

    unsafe fn insert_fixup(&mut self, mut node: *mut Tll<T, U>)
    where
        T: fmt::Debug,
    {
        let sentinel = self.sentinel;

        while !(*(*node).parent).is_black {
            let parent = (*node).parent;
            let Some(gp) = (*node).grandparent() else {
                log::error!(
                    "Parent is black=false but grandparent is null. Node key: {:?}",
                    (*node).key
                );
                break;
            };

            if std::ptr::eq(parent, (*gp).left) {
                let uncle = (*gp).right;

                if !(*uncle).is_black {
                    (*parent).is_black = true;
                    (*uncle).is_black = true;
                    (*gp).is_black = false;
                    node = gp;
                } else {
                    if std::ptr::eq(node, (*parent).right) {
                        node = parent;
                        (*node).rotate_left(sentinel);
                    }

                    // Re-read parent and grandparent after potential rotation
                    let parent = (*node).parent;
                    let grandparent = (*node).grandparent().unwrap();

                    (*parent).is_black = true;
                    (*grandparent).is_black = false;
                    (*grandparent).rotate_right(sentinel);
                }
            } else {
                // Mirror cases for right side
                let uncle = (*gp).left;

                if !(*uncle).is_black {
                    (*parent).is_black = true;
                    (*uncle).is_black = true;
                    (*gp).is_black = false;
                    node = gp;
                } else {
                    if std::ptr::eq(node, (*parent).left) {
                        node = parent;
                        (*node).rotate_right(sentinel);
                    }

                    // Re-read after potential rotation
                    let parent = (*node).parent;
                    let grandparent = (*node).grandparent().unwrap();

                    (*parent).is_black = true;
                    (*grandparent).is_black = false;
                    (*grandparent).rotate_left(sentinel);
                }
            }
        }

        // Ensure root is black
        (*(*sentinel).parent).is_black = true;
    }

    unsafe fn update_sentinel_min_max(&mut self) {
        let sentinel = self.sentinel;
        let root = (*sentinel).parent;

        if (*root).is_sentinel {
            // Empty tree
            (*sentinel).left = sentinel;
            (*sentinel).right = sentinel;
            return;
        }

        // Find leftmost (minimum)
        let mut min = root;
        while !(*(*min).left).is_sentinel {
            min = (*min).left;
        }
        (*sentinel).left = min;

        // Find rightmost (maximum)
        let mut max = root;
        while !(*(*max).right).is_sentinel {
            max = (*max).right;
        }
        (*sentinel).right = max;
    }
}

impl<T, U> Tll<T, U> {
    unsafe fn new(key: T, value: U, sentinel: *mut Tll<T, U>) -> *mut Tll<T, U> {
        let node_ptr = libc::malloc(size_of::<Tll<T, U>>()) as *mut Tll<T, U>;

        let node = Tll {
            left: sentinel,
            parent: sentinel,
            right: sentinel,
            is_black: false,
            is_sentinel: false,
            _padding: [0; 6],
            key,
            data: value,
        };

        std::ptr::write(node_ptr, node);
        node_ptr
    }

    unsafe fn rotate_left(&mut self, sentinel: *mut Tll<T, U>) -> *mut Tll<T, U> {
        let x = self.right;
        let y = (*x).left;
        let parent = self.parent;

        // Perform rotation
        (*x).left = self;
        self.right = y;

        // Update parents
        (*x).parent = parent;
        self.parent = x;

        if !(*y).is_sentinel {
            (*y).parent = self;
        }

        if (*parent).is_sentinel {
            // self was the root, update sentinel
            (*sentinel).parent = x;
        } else if std::ptr::eq((*parent).left, self) {
            // self was a left child
            (*parent).left = x;
        } else {
            // self was a right child
            (*parent).right = x;
        }

        x
    }

    unsafe fn rotate_right(&mut self, sentinel: *mut Tll<T, U>) -> *mut Tll<T, U> {
        let x = self.left;
        let y = (*x).right;
        let parent = self.parent;

        // Perform rotation
        (*x).right = self;
        self.left = y;

        // Update parents
        (*x).parent = parent;
        self.parent = x;

        if !(*y).is_sentinel {
            (*y).parent = self;
        }

        if (*parent).is_sentinel {
            // self was the root, update sentinel
            (*sentinel).parent = x;
        } else if std::ptr::eq((*parent).left, self) {
            // self was a left child
            (*parent).left = x;
        } else {
            // self was a right child
            (*parent).right = x;
        }

        x
    }

    unsafe fn grandparent(&self) -> Option<*mut Tll<T, U>> {
        if (*self.parent).is_sentinel || (*(*self.parent).parent).is_sentinel {
            None
        } else {
            Some((*self.parent).parent)
        }
    }
}

impl<T, U> Drop for TllContainer<T, U> {
    fn drop(&mut self) {
        unsafe {
            if !self.sentinel.is_null() {
                std::ptr::drop_in_place(self.sentinel);
                libc::free(self.sentinel as *mut libc::c_void);
            }
        }
    }
}

impl<T, U> From<&mut Tll<T, U>> for Vec<&mut U> {
    fn from(tll: &mut Tll<T, U>) -> Self {
        let mut result = Vec::new();

        unsafe {
            if tll.is_sentinel {
                if tll.parent.is_null() || (*tll.parent).is_sentinel {
                    return Vec::new();
                }

                in_order_traverse(tll.parent, &mut result);
            } else {
                in_order_traverse(tll as *mut Tll<T, U>, &mut result);
            }
        }

        result.into_iter().map(|(_, data)| data).collect()
    }
}

impl<T: Eq + Hash, U> From<&mut Tll<T, U>> for std::collections::HashMap<&T, &mut U> {
    fn from(tll: &mut Tll<T, U>) -> Self {
        let mut result = std::collections::HashMap::new();

        unsafe {
            if tll.is_sentinel {
                if tll.parent.is_null() || (*tll.parent).is_sentinel {
                    return result;
                }

                let mut items = Vec::new();
                in_order_traverse(tll.parent, &mut items);

                for item in items {
                    let key = item.0;
                    let data = item.1;
                    result.insert(key, data);
                }
            } else {
                let mut items = Vec::new();
                in_order_traverse(tll as *mut Tll<T, U>, &mut items);

                for item in items {
                    let key = item.0;
                    let data = item.1;
                    result.insert(key, data);
                }
            }
        }

        result
    }
}

unsafe fn in_order_traverse<T, U>(node: *mut Tll<T, U>, result: &mut Vec<(&T, &mut U)>) {
    if node.is_null() || (*node).is_sentinel {
        return;
    }

    in_order_traverse((*node).left, result);
    let node_ref = &mut *node;
    result.push((&node_ref.key, &mut node_ref.data));
    in_order_traverse((*node).right, result);
}
