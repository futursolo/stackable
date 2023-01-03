//! A type to clone fn once per thread.

use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use thread_local::ThreadLocal;

/// A value that is lazily initialised once per thread.
pub struct ThreadLocalLazy<T: Send> {
    value: Arc<ThreadLocal<T>>,
    create_value: Arc<dyn Send + Sync + Fn() -> T>,
}

impl<T: Send> fmt::Debug for ThreadLocalLazy<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ThreadLocalLazy<_>")
    }
}

impl<T> Clone for ThreadLocalLazy<T>
where
    T: 'static + Send,
{
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            create_value: self.create_value.clone(),
        }
    }
}

impl<T> ThreadLocalLazy<T>
where
    T: 'static + Send,
{
    /// Creates a thread-local lazy value.
    ///
    /// The create function is called once per thread.
    pub fn new<F>(f: F) -> Self
    where
        F: 'static + Send + Fn() -> T,
    {
        let clonable_inner = Arc::new(Mutex::new(f));
        let create_inner = move || -> T {
            let clonable_inner = clonable_inner.lock().expect("failed to lock?");
            clonable_inner()
        };

        Self {
            value: Arc::new(ThreadLocal::new()),
            create_value: Arc::new(create_inner),
        }
    }
}

impl<T> Deref for ThreadLocalLazy<T>
where
    T: 'static + Send,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value.get_or(&*self.create_value)
    }
}
