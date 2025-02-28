pub mod disassemble;
pub mod unreal;

use _image::Image;
use patternsleuth_image::MemoryAccessError;
pub use patternsleuth_image::image as _image;

use futures::{
    channel::oneshot,
    executor::LocalPool,
    future::{BoxFuture, join_all},
};
use futures_scopes::{
    ScopedSpawnExt, SpawnScope,
    relay::{RelayScopeLocalSpawning, new_relay_scope},
};
use patternsleuth_scanner::Pattern;
use std::{
    any::{Any, TypeId},
    borrow::Cow,
    collections::HashMap,
    error::Error,
    sync::{Arc, Mutex},
};

/// Given an iterator of values, returns Ok(value) if all values are equal or Err
pub fn ensure_one<T: std::fmt::Debug + PartialEq>(data: impl IntoIterator<Item = T>) -> Result<T> {
    try_ensure_one(data.into_iter().map(|v| Ok(v)))
}

/// Given an iterator of values, returns Ok(value) if all values are equal or Err
pub fn try_ensure_one<T: std::fmt::Debug + PartialEq>(
    data: impl IntoIterator<Item = Result<T>>,
) -> Result<T> {
    let mut reached_max = false;

    // TODO use a stack vec to eliminate heap allocation
    let mut unique = vec![];
    for value in data.into_iter() {
        let value = value?;
        if !unique.contains(&value) {
            unique.push(value);
        }
        if unique.len() >= 4 {
            reached_max = true;
            break;
        }
    }
    match unique.len() {
        0 => Err(ResolveError::new_msg("expected at least one value")),
        1 => Ok(unique.swap_remove(0)),
        len => Err(ResolveError::new_msg(format!(
            "found {}{len} unique values {unique:X?}",
            if reached_max { ">=" } else { "" }
        ))),
    }
}

pub type Result<T> = std::result::Result<T, ResolveError>;
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct ResolveError {
    context: Vec<String>,
    r#type: ResolveErrorType,
}
impl ResolveError {
    fn new_msg(msg: impl Into<Cow<'static, str>>) -> Self {
        Self {
            context: vec![],
            r#type: ResolveErrorType::Msg(msg.into()),
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub enum ResolveErrorType {
    Msg(Cow<'static, str>),
    MemoryAccessOutOfBounds(MemoryAccessError),
}
impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &self.r#type {
            ResolveErrorType::Msg(msg) => {
                for ctx in self.context.iter().rev() {
                    write!(f, "{ctx}: ")?;
                }
                write!(f, "{msg}")
            }
            ResolveErrorType::MemoryAccessOutOfBounds(err) => err.fmt(f),
        }
    }
}
impl Error for ResolveError {}

impl From<MemoryAccessError> for ResolveError {
    fn from(value: MemoryAccessError) -> Self {
        Self {
            context: vec![],
            r#type: ResolveErrorType::MemoryAccessOutOfBounds(value),
        }
    }
}

#[macro_export]
macro_rules! _bail_out {
    ($msg:expr) => {
        return Err($crate::ResolveError::new_msg($msg));
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
            None => Err(ResolveError::new_msg(msg)),
        }
    }
}

pub struct NamedResolver {
    pub name: &'static str,
    pub getter: fn() -> &'static DynResolverFactory,
}

inventory::collect!(NamedResolver);
pub fn resolvers() -> impl Iterator<Item = &'static NamedResolver> {
    inventory::iter::<NamedResolver>()
}

type DynResolver<'ctx> = BoxFuture<'ctx, Result<Arc<dyn Resolution>>>;
type Resolver<'ctx, T> = BoxFuture<'ctx, Result<T>>;

#[cfg_attr(feature = "serde-resolvers", typetag::serde(tag = "type"))]
pub trait Resolution: std::fmt::Debug + std::any::Any + Send + Sync + Singleton + DynEq {}

/// Allow comparison of dyn Resolution
/// <https://users.rust-lang.org/t/how-to-compare-two-trait-objects-for-equality/88063/3>
pub trait DynEq: Any + DynEqHelper {
    fn as_any(&self) -> &dyn Any;
    fn as_dyn_eq_helper(&self) -> &dyn DynEqHelper;
    fn level_one(&self, arg2: &dyn DynEqHelper) -> bool;

    fn dyn_eq<T: PartialEq + 'static>(&self, other: &T) -> bool
    where
        Self: Sized,
    {
        if let Some(this) = self.as_any().downcast_ref::<T>() {
            this == other
        } else {
            false
        }
    }
}
pub trait DynEqHelper {
    fn level_two(&self, arg1: &dyn DynEq) -> bool;
}
impl<T: Any + PartialEq> DynEq for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_dyn_eq_helper(&self) -> &dyn DynEqHelper {
        self
    }
    fn level_one(&self, arg2: &dyn DynEqHelper) -> bool {
        arg2.level_two(self)
    }
}
impl<T: Any + PartialEq> DynEqHelper for T {
    fn level_two(&self, arg1: &dyn DynEq) -> bool {
        if let Some(other) = arg1.as_any().downcast_ref::<Self>() {
            other.dyn_eq(self)
        } else {
            false
        }
    }
}
impl PartialEq for dyn Resolution {
    fn eq(&self, other: &Self) -> bool {
        self.level_one(other.as_dyn_eq_helper())
    }
}

pub struct DynResolverFactory {
    pub name: &'static str,
    pub factory: for<'ctx> fn(&'ctx AsyncContext<'_, '_>) -> DynResolver<'ctx>,
}

pub struct ResolverFactory<T> {
    pub name: &'static str,
    pub factory: for<'ctx> fn(&'ctx AsyncContext<'_, '_>) -> Resolver<'ctx, T>,
}

pub use ::futures;
pub use ::inventory;
#[cfg(feature = "serde-resolvers")]
pub use ::typetag;

pub trait PleaseAddCollectForMe {}

#[macro_export]
macro_rules! _matcharm_generator {
    ($enum_name_it:ident { $( $img_ident:ident( $img_ty:ty, $img_feature:literal )),* $(,)? }, {$ctx:ident, $name:ident}) => {
        {
            let img = &$ctx.image().image_type;
            let mut res = None;
            $(
                $crate::cfg_image::$img_ident! {
                    if matches!(img, $crate::_image::$enum_name_it::$img_ident(_)) {
                        res = Some($name::$img_ident($ctx).await);
                    }
                }
            )*
            res.unwrap()
        }
    }
}

#[cfg(feature = "image-pe")]
#[macro_export]
macro_rules! _cfg_image_pe { ($($args:tt)*) => { $($args)* } }
#[cfg(not(feature = "image-pe"))]
#[macro_export]
macro_rules! _cfg_image_pe {
    ($($args:tt)*) => {};
}
#[cfg(feature = "image-elf")]
#[macro_export]
macro_rules! _cfg_image_elf { ($($args:tt)*) => { $($args)* } }
#[cfg(not(feature = "image-elf"))]
#[macro_export]
macro_rules! _cfg_image_elf {
    ($($args:tt)*) => {};
}

#[macro_export]
macro_rules! _impl_resolver {
    (all, $name:ident, |$ctx:ident| async $x:block ) => {
        $crate::_impl_resolver_inner!($name, |$ctx| async $x);

        impl $crate::Singleton for $name {
            fn get(&self) -> Option<usize> {
                None
            }
        }
    };

    ($arch:ident, $name:ident, |$ctx:ident| async $x:block ) => {
        $crate::cfg_image::$arch! {
            impl $name where $name: $crate::PleaseAddCollectForMe {
                #[allow(non_snake_case)]
                pub async fn $arch($ctx: &$crate::AsyncContext<'_, '_>) -> $crate::Result<$name> $x
            }
        }
    };

    (collect, $name:ident) => {
        $crate::_impl_resolver_inner!($name, |ctx| async {
            $crate::_image::image_type_reflection!(all, impl_resolver; generate; {ctx, $name})
        });

        impl $crate::Singleton for $name {
            fn get(&self) -> Option<usize> {
                None
            }
        }

        impl $crate::PleaseAddCollectForMe for $name {}
    };

    (generate, $enum_name_it:ident { $( $img_ident:ident( $img_ty:ty, $img_feature:literal )),* $(,)? }, {$ctx:ident, $name:ident}) => {
        $crate::matcharm_generator!(
            $enum_name_it { $( $img_ident( $img_ty, $img_feature )),* },
            { $ctx, $name }
        )
    };
}

#[macro_export]
macro_rules! _impl_resolver_singleton {
    (all, $name:ident, |$ctx:ident| async $x:block ) => {
        $crate::_impl_resolver_inner!($name, |$ctx| async {
            if let Some(a) = std::env::var(concat!("PATTERNSLEUTH_RES_", stringify!($name))).ok().and_then(|s| (s.strip_prefix("0x").map(|s| usize::from_str_radix(s, 16).ok()).unwrap_or_else(|| s.parse().ok()))) {
                return Ok($name(a));
            }
            $x
        });

        impl $crate::Singleton for $name {
            fn get(&self) -> Option<usize> {
                Some(self.0)
            }
        }
    };

    ($arch:ident, $name:ident, |$ctx:ident| async $x:block ) => {
        $crate::cfg_image::$arch! {
            impl $name where $name: $crate::PleaseAddCollectForMe {
                #[allow(non_snake_case)]
                async fn $arch($ctx: &$crate::AsyncContext<'_, '_>) -> $crate::Result<$name> $x
            }
        }
    };

    (collect, $name:ident) => {
        $crate::_impl_resolver_inner!($name, |ctx| async {
            if let Some(a) = std::env::var(concat!("PATTERNSLEUTH_RES_", stringify!($name))).ok().and_then(|s| (s.strip_prefix("0x").map(|s| usize::from_str_radix(s, 16).ok()).unwrap_or_else(|| s.parse().ok()))) {
                return Ok($name(a));
            }
            $crate::_image::image_type_reflection!(all, impl_resolver_singleton; generate; {ctx, $name})
        });

        impl $crate::Singleton for $name {
            fn get(&self) -> Option<usize> {
                Some(self.0)
            }
        }

        impl $crate::PleaseAddCollectForMe for $name {}
    };

    (generate, $enum_name_it:ident { $( $img_ident:ident( $img_ty:ty, $img_feature:literal )),* $(,)? }, {$ctx:ident, $name:ident}) => {
        $crate::matcharm_generator!(
            $enum_name_it { $( $img_ident( $img_ty, $img_feature )),* },
            { $ctx, $name }
        )
    };
}
#[macro_export]
macro_rules! _impl_resolver_inner {
    ( $name:ident, |$ctx:ident| async $x:block ) => {
        $crate::inventory::submit! {
            $crate::NamedResolver { name: stringify!($name), getter: $name::dyn_resolver }
        }

        #[cfg_attr(feature = "serde-resolvers", $crate::typetag::serde)]
        impl $crate::Resolution for $name {}

        impl $name {
            pub fn resolver() -> &'static $crate::ResolverFactory<$name> {
                static GLOBAL: ::std::sync::OnceLock<&$crate::ResolverFactory<$name>> = ::std::sync::OnceLock::new();

                GLOBAL.get_or_init(|| &$crate::ResolverFactory {
                    name: stringify!($name),
                    factory: |$ctx: &$crate::AsyncContext| -> $crate::futures::future::BoxFuture<$crate::Result<$name>> {
                        Box::pin(async $x)
                    },
                })
            }
            pub fn dyn_resolver() -> &'static $crate::DynResolverFactory {
                static GLOBAL: ::std::sync::OnceLock<&$crate::DynResolverFactory> = ::std::sync::OnceLock::new();

                GLOBAL.get_or_init(|| &$crate::DynResolverFactory {
                    name: stringify!($name),
                    factory: |$ctx: &$crate::AsyncContext| -> $crate::futures::future::BoxFuture<$crate::Result<::std::sync::Arc<dyn $crate::Resolution>>> {
                        Box::pin(async {
                            $ctx.resolve(Self::resolver()).await.map(|ok| -> ::std::sync::Arc<dyn $crate::Resolution> { ok })
                        })
                    },
                })
            }
        }
    };
}

#[macro_export]
macro_rules! _impl_try_collector {
    (
        $(#[$outer:meta])*
        $struct_vis:vis struct $struct_name:ident {
            $(
                $(#[$inner:ident $($args:tt)*])*
                $member_vis:vis $member_name:ident: $resolver:path,
            )*
        }
    ) => {
        #[allow(non_snake_case)]
        $(#[$outer])*
        $struct_vis struct $struct_name {
            $(
                $(#[$inner $($args)*])*
                $member_vis $member_name: ::std::sync::Arc<$resolver>,
            )*
        }
        $crate::_impl_resolver!(all, $struct_name, |ctx| async {
            #[allow(non_snake_case)]
            let (
                $( $member_name, )*
            ) = $crate::futures::try_join!(
                $( ctx.resolve($resolver::resolver()), )*
            )?;
            Ok($struct_name {
                $( $member_name, )*
            })
        });
    };
}

#[macro_export]
macro_rules! _impl_collector {
    (
        $(#[$outer:meta])*
        $struct_vis:vis struct $struct_name:ident {
            $(
                $(#[$inner:ident $($args:tt)*])*
                $member_vis:vis $member_name:ident: $resolver:path,
            )*
        }
    ) => {
        #[allow(non_snake_case)]
        $(#[$outer])*
        $struct_vis struct $struct_name {
            $(
                $(#[$inner $($args)*])*
                $member_vis $member_name: $crate::Result<::std::sync::Arc<$resolver>>,
            )*
        }
        $crate::_impl_resolver!(all, $struct_name, |ctx| async {
            #[allow(non_snake_case)]
            let (
                $( $member_name, )*
            ) = $crate::futures::join!(
                $( ctx.resolve($resolver::resolver()), )*
            );
            Ok($struct_name {
                $( $member_name, )*
            })
        });
    };
}

pub use _impl_collector as impl_collector;
pub use _impl_resolver as impl_resolver;
pub use _impl_resolver_singleton as impl_resolver_singleton;
pub use _impl_try_collector as impl_try_collector;
pub use _matcharm_generator as matcharm_generator;
pub mod cfg_image {
    pub use _cfg_image_elf as ElfImage;
    pub use _cfg_image_pe as PEImage;
}

pub trait Singleton {
    fn get(&self) -> Option<usize>;
}

type AnyValue = Result<Arc<dyn Any + Send + Sync>>;

#[derive(Debug)]
struct PatternMatches {
    pattern: Pattern,
    matches: Vec<usize>,
}

#[derive(Default)]
struct AsyncContextInnerWrite {
    resolvers: HashMap<TypeId, AnyValue>,
    pending_resolvers: HashMap<TypeId, Vec<oneshot::Sender<AnyValue>>>,
    queue: Vec<(Pattern, oneshot::Sender<PatternMatches>)>,
}

struct AsyncContextInnerRead<'img, 'data> {
    write: Mutex<AsyncContextInnerWrite>,
    image: &'img Image<'data>,
}

#[derive(Clone)]
pub struct AsyncContext<'img, 'data> {
    read: Arc<AsyncContextInnerRead<'img, 'data>>,
}

impl<'img, 'data> AsyncContext<'img, 'data> {
    fn new(image: &'img Image<'data>) -> Self {
        Self {
            read: Arc::new(AsyncContextInnerRead {
                write: Default::default(),
                image,
            }),
        }
    }
    pub fn image(&self) -> &'img Image<'data> {
        self.read.image
    }
    pub async fn scan(&self, pattern: Pattern) -> Vec<usize> {
        self.scan_tagged((), pattern).await.2
    }
    pub async fn scan_tagged2<T: Copy>(&self, tag: T, pattern: Pattern) -> Vec<(T, usize)> {
        self.scan_tagged(tag, pattern)
            .await
            .2
            .into_iter()
            .map(|a| (tag, a))
            .collect()
    }
    pub async fn scan_tagged<T>(&self, tag: T, pattern: Pattern) -> (T, Pattern, Vec<usize>) {
        let (tx, rx) = oneshot::channel::<PatternMatches>();
        {
            let mut lock = self.read.write.lock().unwrap();
            lock.queue.push((pattern, tx));
        }
        let PatternMatches { pattern, matches } = rx.await.unwrap();
        (tag, pattern, matches)
    }
    pub async fn resolve<T: Send + Sync + 'static>(
        &self,
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
        let name = resolver.name;
        let resolver = (resolver.factory)(self);
        let res = match resolver.await {
            Err(mut e) => {
                e.context.push(name.into());
                Err(e)
            }
            res => res,
        };
        let res = res.map(Arc::new);

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

#[tracing::instrument(level = "debug", skip_all, fields(stages))]
pub fn eval<'img, 'data, F, T: Send + Sync>(image: &'img Image<'data>, f: F) -> T
where
    F: for<'ctx> FnOnce(&'ctx AsyncContext<'_, '_>) -> BoxFuture<'ctx, T> + Send + Sync,
{
    {
        tracing::debug!("starting eval");

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

        let mut i = 0;

        loop {
            i += 1;

            tracing::debug_span!("resolvers", stage = i).in_scope(|| {
                pool.run_until_stalled();
            });

            if let Ok(res) = tx.try_recv() {
                tracing::Span::current().record("stages", i);
                break res;
            } else {
                let queue: Vec<_> = std::mem::take(&mut ctx.read.write.lock().unwrap().queue);
                let (patterns, rx): (Vec<_>, Vec<_>) = queue.into_iter().unzip();
                let setup = patterns.iter().collect::<Vec<_>>();

                let span = tracing::debug_span!("patterns", patterns = setup.len()).entered();
                for p in &setup {
                    tracing::debug!("pattern = {p:?}");
                }

                let mut all_results = rx.into_iter().map(|rx| (rx, vec![])).collect::<Vec<_>>();

                for section in image.memory.sections() {
                    let span = tracing::debug_span!(
                        "section",
                        section = section.name(),
                        //kind = format!("{:?}", section.kind()),
                        results = tracing::field::Empty
                    )
                    .entered();

                    let base_address = section.address();
                    let data = section.data();

                    let scan_results =
                        patternsleuth_scanner::scan_pattern(&setup, base_address, data);

                    let mut total = 0;

                    for (i, res) in scan_results.iter().enumerate() {
                        total += res.len();
                        all_results[i].1.extend(res)
                    }

                    span.record("results", total);
                }

                drop(span);

                for ((rx, matches), pattern) in all_results.into_iter().zip(patterns) {
                    rx.send(PatternMatches { pattern, matches }).unwrap();
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
