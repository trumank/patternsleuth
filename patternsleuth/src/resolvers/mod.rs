pub mod unreal;

use crate::{Image, Memory, ResolutionAction, ResolutionType, ResolveContext, ResolveStages};
use futures::{
    channel::oneshot,
    executor::LocalPool,
    future::{join_all, BoxFuture},
};
use futures_scopes::{
    relay::{new_relay_scope, RelayScopeLocalSpawning},
    ScopedSpawnExt, SpawnScope,
};
use patternsleuth_scanner::Pattern;
use std::{
    any::{Any, TypeId},
    borrow::Cow,
    collections::HashMap,
    error::Error,
    sync::{Arc, Mutex},
};

/// Simply return address of match
pub fn resolve_self(ctx: ResolveContext, _stages: &mut ResolveStages) -> ResolutionAction {
    ResolutionType::Address(ctx.match_address).into()
}

/// Return containing function via exception table lookup
pub fn resolve_function(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
    stages.0.push(ctx.match_address);
    ctx.exe
        .get_root_function(ctx.match_address)
        .map(|f| f.range.start)
        .into()
}

fn resolve_rip(
    memory: &Memory,
    match_address: usize,
    next_opcode_offset: usize,
    stages: &mut ResolveStages,
) -> ResolutionAction {
    stages.0.push(match_address);
    let rip_relative_value_address = match_address;
    // calculate the absolute address from the RIP relative value.
    let address = rip_relative_value_address
        .checked_add_signed(i32::from_le_bytes(
            memory[rip_relative_value_address..rip_relative_value_address + 4]
                .try_into()
                .unwrap(),
        ) as isize)
        .map(|a| a + next_opcode_offset);
    address.into()
}

/// Resolve RIP address at match, accounting for `N` bytes to the end of the instruction (usually 4)
pub fn resolve_rip_offset<const N: usize>(
    ctx: ResolveContext,
    stages: &mut ResolveStages,
) -> ResolutionAction {
    resolve_rip(ctx.memory, ctx.match_address, N, stages)
}

/// Given an iterator of values, returns Ok(value) if all values are equal or Err
pub fn ensure_one<T: PartialEq>(data: impl IntoIterator<Item = T>) -> Result<T> {
    let mut iter = data.into_iter();
    let first = iter.next().context("expected at least one value")?;
    for value in iter {
        if value != first {
            bail_out!("iter returned multiple unique values");
        }
    }
    Ok(first)
}

pub type Result<T> = std::result::Result<T, ResolveError>;
#[derive(Debug, Clone)]
pub enum ResolveError {
    Msg(Cow<'static, str>),
}
impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ResolveError::Msg(msg) => write!(f, "{msg}"),
        }
    }
}
impl Error for ResolveError {}

#[macro_export]
macro_rules! _bail_out {
    ($msg:literal) => {
        return Err($crate::resolvers::ResolveError::Msg($msg.into()));
    };
}
pub use _bail_out as bail_out;

pub trait Context<T>
where
    Self: Sized,
{
    fn context(self, msg: &'static str) -> Result<T>;
}
impl<T> Context<T> for Option<T> {
    fn context(self, msg: &'static str) -> Result<T> {
        match self {
            Some(value) => Ok(value),
            None => Err(ResolveError::Msg(msg.into())),
        }
    }
}

type DynResolverFactoryGetter = (&'static str, fn() -> &'static DynResolverFactory);

type DynResolver<'ctx> = BoxFuture<'ctx, Result<Arc<dyn Resolution>>>;
type Resolver<'ctx, T> = BoxFuture<'ctx, Result<T>>;

pub trait Resolution: std::fmt::Debug + Send + Sync {}
impl<T: std::fmt::Debug + Send + Sync> Resolution for T {}
pub struct DynResolverFactory {
    pub factory: for<'ctx> fn(&'ctx AsyncContext<'_>) -> DynResolver<'ctx>,
}

pub struct ResolverFactory<T> {
    pub factory: for<'ctx> fn(&'ctx AsyncContext<'_>) -> Resolver<'ctx, T>,
}

pub use ::futures;

#[macro_export]
macro_rules! _impl_resolver {
    ( $name:ident, |$ctx:ident| async $x:block ) => {
        impl $name {
            pub fn resolver() -> &'static $crate::resolvers::ResolverFactory<$name> {
                static GLOBAL: ::std::sync::OnceLock<&$crate::resolvers::ResolverFactory<$name>> = ::std::sync::OnceLock::new();

                GLOBAL.get_or_init(|| &$crate::resolvers::ResolverFactory {
                    factory: |$ctx: &$crate::resolvers::AsyncContext| -> $crate::resolvers::futures::future::BoxFuture<$crate::resolvers::Result<$name>> {
                        Box::pin(async $x)
                    },
                })
            }
            pub fn dyn_resolver() -> &'static $crate::resolvers::DynResolverFactory {
                static GLOBAL: ::std::sync::OnceLock<&$crate::resolvers::DynResolverFactory> = ::std::sync::OnceLock::new();

                GLOBAL.get_or_init(|| &$crate::resolvers::DynResolverFactory {
                    factory: |$ctx: &$crate::resolvers::AsyncContext| -> $crate::resolvers::futures::future::BoxFuture<$crate::resolvers::Result<::std::sync::Arc<dyn $crate::resolvers::Resolution>>> {
                        Box::pin(async {
                            $ctx.resolve(Self::resolver()).await.map(|ok| -> ::std::sync::Arc<dyn $crate::resolvers::Resolution> { ::std::sync::Arc::new(ok) })
                        })
                    },
                })
            }
        }
    };
}
pub use _impl_resolver as impl_resolver;

type AnyValue = Result<Arc<dyn Any + Send + Sync>>;

#[derive(Default)]
struct AsyncContextInnerWrite {
    resolvers: HashMap<TypeId, AnyValue>,
    pending_resolvers: HashMap<TypeId, Vec<oneshot::Sender<AnyValue>>>,
    queue: Vec<(Pattern, oneshot::Sender<Vec<usize>>)>,
}

struct AsyncContextInnerRead<'data> {
    write: Mutex<AsyncContextInnerWrite>,
    image: &'data Image<'data>,
}

#[derive(Clone)]
pub struct AsyncContext<'data> {
    read: Arc<AsyncContextInnerRead<'data>>,
}

impl<'data> AsyncContext<'data> {
    fn new(image: &'data Image<'data>) -> Self {
        Self {
            read: Arc::new(AsyncContextInnerRead {
                write: Default::default(),
                image,
            }),
        }
    }
    pub fn image(&self) -> &Image<'_> {
        self.read.image
    }
    async fn scan(&self, pattern: Pattern) -> Vec<usize> {
        self.scan_tagged((), pattern).await.1
    }
    async fn scan_tagged<T>(&self, tag: T, pattern: Pattern) -> (T, Vec<usize>) {
        let (tx, rx) = oneshot::channel::<Vec<usize>>();
        {
            let mut lock = self.read.write.lock().unwrap();
            lock.queue.push((pattern, tx));
        }
        (tag, rx.await.unwrap())
    }
    pub async fn resolve<'ctx, T: Send + Sync + 'static>(
        &'ctx self,
        resolver: &ResolverFactory<T>,
    ) -> Result<Arc<T>> {
        let t = TypeId::of::<T>();
        let rx = {
            // first check to see if we've already computed the resolver
            let mut lock = self.read.write.lock().unwrap();
            if let Some(res) = lock.resolvers.get(&t) {
                return res.clone().map(|ok| ok.downcast::<T>().unwrap());
            }

            // no value found so check if there is a pending resolver for the same type
            if let Some(res) = lock.pending_resolvers.get_mut(&t) {
                // there is, so wait for it to complete by adding a channel
                let (tx, rx) = oneshot::channel::<AnyValue>();
                res.push(tx);

                Some(rx)
            } else {
                // TODO may be possible to used a shared future instead
                // https://docs.rs/futures/latest/futures/future/trait.FutureExt.html#method.shared
                // we're the future that is computing the resolver so init the listener vec
                lock.pending_resolvers.entry(t).or_default();
                None
            }
        };

        // some convoluted logic to drop the lock to make the future `Send`
        if let Some(rx) = rx {
            return rx.await.unwrap().map(|ok| ok.downcast::<T>().unwrap());
        }

        // compute the resolver value
        let resolver = (resolver.factory)(self);
        let res = resolver.await.map(Arc::new);

        let cache: Result<Arc<dyn Any + Send + Sync>> = match res.as_ref() {
            Ok(ok) => Ok(ok.clone()),
            Err(e) => Err(e.clone()),
        };

        // insert new value
        let mut lock = self.read.write.lock().unwrap();
        lock.resolvers.insert(t, cache.clone());

        // update any other listening futures
        for tx in lock.pending_resolvers.remove(&t).unwrap() {
            tx.send(cache.clone()).unwrap();
        }

        res
    }
}

pub fn eval<F, T: Send + Sync>(image: &Image<'_>, f: F) -> T
where
    F: for<'ctx> FnOnce(&'ctx AsyncContext<'_>) -> BoxFuture<'ctx, T> + Send + Sync,
{
    {
        let ctx = AsyncContext::new(image);
        let (rx, tx) = std::sync::mpsc::channel();

        let scope = new_relay_scope!();
        let mut pool = LocalPool::new();
        let _ = pool.spawner().spawn_scope(scope);

        scope
            .spawner()
            .spawn_scoped({
                let ctx = ctx.clone();
                async move {
                    rx.send(f(&ctx).await).unwrap();
                }
            })
            .unwrap();

        loop {
            pool.run_until_stalled();

            if let Ok(res) = tx.try_recv() {
                break res;
            } else {
                let queue: Vec<_> = std::mem::take(&mut ctx.read.write.lock().unwrap().queue);
                let (patterns, rx): (Vec<_>, Vec<_>) = queue.into_iter().unzip();
                let setup = patterns.iter().collect::<Vec<_>>();

                let mut all_results = rx.into_iter().map(|rx| (rx, vec![])).collect::<Vec<_>>();

                for section in image.memory.sections() {
                    let base_address = section.address();
                    let data = section.data();

                    let scan_results =
                        patternsleuth_scanner::scan_pattern(&setup, base_address, data);

                    for (i, res) in scan_results.iter().enumerate() {
                        all_results[i].1.extend(res)
                    }
                }

                for (rx, results) in all_results {
                    rx.send(results).unwrap();
                }
            }
        }
    }
}

pub fn resolve<T: Send + Sync>(
    image: &Image<'_>,
    resolver: &'static ResolverFactory<T>,
) -> Result<T> {
    eval(image, |ctx| Box::pin(async { ctx.resolve(resolver).await }))
        .map(|ok| Arc::<T>::into_inner(ok).unwrap())
}

pub fn resolve_many(
    image: &Image<'_>,
    resolvers: &[fn() -> &'static DynResolverFactory],
) -> Vec<Result<Arc<dyn Resolution>>> {
    let fns = resolvers.iter().map(|r| r().factory).collect::<Vec<_>>();
    eval(image, |ctx| {
        Box::pin(async { join_all(fns.into_iter().map(|f| f(ctx))).await })
    })
}
