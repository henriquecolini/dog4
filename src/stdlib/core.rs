//! Output, conversions, errors and introspection.

use super::{Library, checked_str, items, join_display};
use crate::error::Error;
use crate::value::Value;

pub(super) fn install(lib: &mut Library) {
    // `put x` outputs x as is; `put a b c` outputs their text joined by spaces.
    lib.def("put", "%args", |vm, args| {
        let values = items(&args[0]);
        match values {
            [] => Ok(Value::Null),
            [one] => Ok(one.clone()),
            many => checked_str(vm, join_display(many, " ")),
        }
    });
    lib.def("pln", "%args", |vm, args| {
        let mut line = join_display(items(&args[0]), " ");
        line.push('\n');
        checked_str(vm, line)
    });
    lib.def("type", "value", |_, args| {
        Ok(Value::str(args[0].type_name()))
    });
    // Conversions are just typed identity functions: the parameter type coerces.
    lib.def("num", "value: num", |_, args| {
        Ok(std::mem::take(&mut args[0]))
    });
    lib.def("str", "value: str", |_, args| {
        Ok(std::mem::take(&mut args[0]))
    });
    lib.def("bool", "value: bool", |_, args| {
        Ok(std::mem::take(&mut args[0]))
    });
    lib.def("arr", "value: arr", |_, args| {
        Ok(std::mem::take(&mut args[0]))
    });
    lib.def("src", "function", |vm, args| {
        let function = match &args[0] {
            Value::Func(f) => f.clone(),
            Value::Str(name) => match vm.lookup(name) {
                Some(Value::Func(f)) => f,
                Some(other) => {
                    return Err(Error::type_error(format!(
                        "`{name}` is {}, not a function",
                        crate::value::with_article(other.type_name())
                    )));
                }
                None => return Err(Error::name_error(format!("`{name}` is not defined"))),
            },
            other => {
                return Err(Error::type_error(format!(
                    "can't show the source of {}",
                    other.describe()
                )));
            }
        };
        Ok(Value::str(function.source()))
    });
    lib.def("fail", "%message", |_, args| {
        Err(match items(&args[0]) {
            [one] => Error::thrown(one.clone()),
            many => Error::thrown(Value::str(join_display(many, " "))),
        })
    });
    lib.def("assert", "condition", |_, args| {
        if args[0].truthy() {
            Ok(Value::Null)
        } else {
            Err(Error::thrown(Value::str("assertion failed")))
        }
    });
    lib.def("assert", "condition, %message", |_, args| {
        if args[0].truthy() {
            Ok(Value::Null)
        } else {
            Err(Error::thrown(Value::str(join_display(
                items(&args[1]),
                " ",
            ))))
        }
    });
}

pub(super) fn install_print(lib: &mut Library) {
    lib.def("print", "%args", |vm, args| {
        let line = join_display(items(&args[0]), " ");
        vm.print(&line)?;
        Ok(Value::Null)
    });
    lib.def("println", "%args", |vm, args| {
        let mut line = join_display(items(&args[0]), " ");
        line.push('\n');
        vm.print(&line)?;
        Ok(Value::Null)
    });
}

pub(super) fn install_eval(lib: &mut Library) {
    lib.def("eval", "code", |vm, args| {
        let code = match &args[0] {
            Value::Str(s) => s.to_string(),
            other => other.to_string(),
        };
        vm.eval(&code)
    });
}
