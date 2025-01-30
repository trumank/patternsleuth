/// Define a set of functions that dispatch to the appropriate image type as its inner type
/// @define_imagetype accepts enum name and its variants inside a block, and defines the enum
/// @define_matcharm accepts the enum name and its variants inside a block, self to avoid hygienic issues, the function name, and the function arguments
macro_rules! image_type_dispatch {
    (
        @enum $enum_name_it:ident as $enum_name_macro_it:ident $enum_tt:tt
        @fns {
            $(fn $fnname_it:ident($($arg:ident: $arg_ty:ty),*) -> $ret:ty);* $(;)?
        }
    ) => {
        image_type_dispatch!(@define_imagetype $enum_name_it $enum_tt);
        #[allow(unused)]
        impl<'data> Image<'data> {
            $(
                pub fn $fnname_it(&self, $($arg: $arg_ty),*) -> $ret {
                    image_type_dispatch!(@define_matcharm $enum_name_it $enum_tt, self, $fnname_it, $($arg),*)
                }
            )*
        }
        image_type_dispatch!(@generate_macro_for_enum $enum_name_it $enum_name_macro_it $enum_tt);
    };
    (@define_imagetype $enum_name_it:ident { $( $img_ident:ident( $img_ty:ty, $img_feature:literal )),* $(,)? }) => {
        pub enum $enum_name_it {
            $(
                #[cfg(feature = $img_feature)]
                $img_ident($img_ty),
            )*
        }
    };
    (@define_matcharm $enum_name_it:ident { $( $img_ident:ident( $img_ty:ty, $img_feature:literal )),* $(,)? }, $self:ident, $fnname_it:ident, $args_tt:tt) => {
        match &$self.image_type {
            $(
                #[cfg(feature = $img_feature)]
                $enum_name_it::$img_ident(inner) => inner.$fnname_it($self, $args_tt),
            )*
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    };

    (@define_matcharm $enum_name_it:ident { $( $img_ident:ident( $img_ty:ty, $img_feature:literal )),* $(,)? }, $self:ident, $fnname_it:ident, ) => {
        match &$self.image_type {
            $(
                #[cfg(feature = $img_feature)]
                $enum_name_it::$img_ident(inner) => inner.$fnname_it($self),
            )*
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    };

    (@generate_macro_for_enum $enum_name_it:ident $enum_name_macro_it:ident { $( $img_ident:ident( $img_ty:ty, $img_feature:literal )),* $(,)? }) => {
        #[allow(unused_macros)]
        #[macro_export]
        macro_rules! $enum_name_macro_it {
            (all, $macroname:ident; $id:ident; $arg:tt) => {
                $macroname!($id, $enum_name_it {$( $img_ident($img_ty, $img_feature),)*}, $arg)
            };
        }

        #[allow(unused_imports)]
        pub(crate) use $enum_name_macro_it;
    };
}

pub(crate) use image_type_dispatch;
