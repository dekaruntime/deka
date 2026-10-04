//! Native implementation of the closed math module and number receiver methods.
use crate::{HostOp, HostReply, HostType, HostValue, Hosts, Result};
// One source file serves the checker and compiler-free runtime alike.
#[path = "../../deka_syntax/src/math_catalog.rs"]
mod math_catalog;

pub const PI: f64 = std::f64::consts::PI;

pub(crate) fn operation(name: &str) -> String {
    let method = math_catalog::method(name).expect("catalog math method");
    format!("number_math_{}", method.name)
}
#[derive(Clone, Copy)]
enum Body {
    Unary(fn(f64) -> f64),
    Binary(fn(f64, f64) -> f64),
}
fn round(x: f64) -> f64 {
    if !x.is_finite() || x == 0. {
        return x;
    }
    let floor = x.floor();
    let value = if x - floor < 0.5 { floor } else { x.ceil() };
    if value == 0. {
        0.0_f64.copysign(x)
    } else {
        value
    }
}
fn sign(x: f64) -> f64 {
    if x == 0. || x.is_nan() { x } else { x.signum() }
}
fn min(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x == 0. && y == 0. {
        if x.is_sign_negative() || y.is_sign_negative() {
            -0.
        } else {
            0.
        }
    } else {
        x.min(y)
    }
}
fn max(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x == 0. && y == 0. {
        if x.is_sign_negative() && y.is_sign_negative() {
            -0.
        } else {
            0.
        }
    } else {
        x.max(y)
    }
}
fn pow(x: f64, y: f64) -> f64 {
    if y == 0. {
        1.
    } else if x.abs() == 1. && y.is_infinite() {
        f64::NAN
    } else {
        x.powf(y)
    }
}
fn body(name: &str) -> Option<Body> {
    Some(match name {
        "abs" => Body::Unary(f64::abs),
        "ceil" => Body::Unary(f64::ceil),
        "floor" => Body::Unary(f64::floor),
        "round" => Body::Unary(round),
        "trunc" => Body::Unary(f64::trunc),
        "sign" => Body::Unary(sign),
        "cbrt" => Body::Unary(f64::cbrt),
        "exp" => Body::Unary(f64::exp),
        "atan" => Body::Unary(f64::atan),
        "sinh" => Body::Unary(f64::sinh),
        "cosh" => Body::Unary(f64::cosh),
        "tanh" => Body::Unary(f64::tanh),
        "sqrt" => Body::Unary(f64::sqrt),
        "log" => Body::Unary(f64::ln),
        "log2" => Body::Unary(f64::log2),
        "log10" => Body::Unary(f64::log10),
        "asin" => Body::Unary(f64::asin),
        "acos" => Body::Unary(f64::acos),
        "acosh" => Body::Unary(f64::acosh),
        "atanh" => Body::Unary(f64::atanh),
        "sin" => Body::Unary(f64::sin),
        "cos" => Body::Unary(f64::cos),
        "tan" => Body::Unary(f64::tan),
        "max" => Body::Binary(max),
        "min" => Body::Binary(min),
        "pow" => Body::Binary(pow),
        _ => return None,
    })
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    for method in math_catalog::methods() {
        let implementation = body(method.name)
            .ok_or_else(|| format!("missing native math operation {}", method.name))?;
        let expected_arguments = match implementation {
            Body::Unary(_) => 0,
            Body::Binary(_) => 1,
        };
        if expected_arguments != method.arguments {
            return Err("native math implementation disagrees with catalog arity".into());
        }
        let result = if method.partial {
            HostType::Option(Box::new(HostType::Number))
        } else {
            HostType::Number
        };
        hosts.register(HostOp::new(
            &operation(method.name),
            vec![HostType::Number; 1 + method.arguments],
            result,
            false,
            move |args| {
                let HostValue::Number(x) = args[0] else {
                    unreachable!("validated math argument")
                };
                let result = match implementation {
                    Body::Unary(f) => f(x),
                    Body::Binary(f) => {
                        let HostValue::Number(y) = args[1] else {
                            unreachable!("validated math argument")
                        };
                        f(x, y)
                    }
                };
                HostReply::Ready(Ok(if method.partial {
                    HostValue::Option(if result.is_nan() {
                        None
                    } else {
                        Some(Box::new(HostValue::Number(result)))
                    })
                } else {
                    HostValue::Number(result)
                }))
            },
        ))?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rounding_ties_and_signed_zero_match_the_shipped_number_contract() {
        assert_eq!(round(-1.5), -1.);
        assert_eq!(round(1.5), 2.);
        assert!(round(-0.5).is_sign_negative());
        assert!(sign(-0.).is_sign_negative());
        assert!(min(0., -0.).is_sign_negative());
        assert!(!max(-0., 0.).is_sign_negative());
        assert!(min(f64::NAN, 1.).is_nan());
        assert!(max(1., f64::NAN).is_nan());
        assert!(pow(1., f64::INFINITY).is_nan());
        assert_eq!(pow(f64::NAN, 0.), 1.);
        assert_eq!(round(4503599627370497.), 4503599627370497.);
    }
}
