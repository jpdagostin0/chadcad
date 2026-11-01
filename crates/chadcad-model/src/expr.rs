//! Dimension expressions: `width / 2 + sqrt(depth) * 3`.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A dimension: source text, evaluated against the parameter table at build time.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Expr(pub String);

impl Default for Expr {
    fn default() -> Self {
        Self("0".into())
    }
}

impl fmt::Debug for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<f64> for Expr {
    fn from(v: f64) -> Self {
        Self(v.to_string())
    }
}

impl From<&str> for Expr {
    fn from(v: &str) -> Self {
        Self(v.to_owned())
    }
}

impl Serialize for Expr {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Expr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Expr;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a number or an expression string")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Expr, E> {
                Ok(Expr(v.to_owned()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Expr, E> {
                Ok(Expr::from(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Expr, E> {
                Ok(Expr(v.to_string()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Expr, E> {
                Ok(Expr(v.to_string()))
            }
        }
        d.deserialize_any(V)
    }
}

/// Evaluate `src`, resolving identifiers through `lookup`.
pub fn eval(src: &str, lookup: &mut dyn FnMut(&str) -> Result<f64, String>) -> Result<f64, String> {
    let mut p = Parser { src: src.as_bytes(), at: 0, lookup };
    let v = p.sum()?;
    p.space();
    if p.at != p.src.len() {
        return Err(format!("unexpected input at column {} in `{src}`", p.at + 1));
    }
    if !v.is_finite() {
        return Err(format!("`{src}` is not a finite number"));
    }
    Ok(v)
}

struct Parser<'a> {
    src: &'a [u8],
    at: usize,
    lookup: &'a mut dyn FnMut(&str) -> Result<f64, String>,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.src.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.space();
        let hit = self.src.get(self.at) == Some(&c);
        self.at += usize::from(hit);
        hit
    }

    fn sum(&mut self) -> Result<f64, String> {
        let mut v = self.product()?;
        loop {
            if self.eat(b'+') {
                v += self.product()?;
            } else if self.eat(b'-') {
                v -= self.product()?;
            } else {
                return Ok(v);
            }
        }
    }

    fn product(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            if self.eat(b'*') {
                v *= self.unary()?;
            } else if self.eat(b'/') {
                v /= self.unary()?;
            } else if self.eat(b'%') {
                v %= self.unary()?;
            } else {
                return Ok(v);
            }
        }
    }

    fn unary(&mut self) -> Result<f64, String> {
        if self.eat(b'-') {
            return Ok(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.atom()?;
        if self.eat(b'^') {
            return Ok(base.powf(self.unary()?));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<f64, String> {
        self.space();
        if self.eat(b'(') {
            let v = self.sum()?;
            if !self.eat(b')') {
                return Err("missing `)`".into());
            }
            return Ok(v);
        }
        let start = self.at;
        match self.src.get(self.at) {
            Some(c) if c.is_ascii_digit() || *c == b'.' => {
                while let Some(c) = self.src.get(self.at) {
                    let exponent_sign = matches!(c, b'+' | b'-')
                        && matches!(self.src.get(self.at.wrapping_sub(1)), Some(b'e' | b'E'));
                    if !(c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E') || exponent_sign) {
                        break;
                    }
                    self.at += 1;
                }
                let text = std::str::from_utf8(&self.src[start..self.at]).unwrap_or("");
                text.parse().map_err(|_| format!("bad number `{text}`"))
            }
            Some(c) if c.is_ascii_alphabetic() || *c == b'_' => {
                while self.src.get(self.at).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    self.at += 1;
                }
                let name = std::str::from_utf8(&self.src[start..self.at]).unwrap_or("").to_owned();
                if self.eat(b'(') { self.call(&name) } else { self.name(&name) }
            }
            Some(c) => Err(format!("unexpected `{}`", *c as char)),
            None => Err("unexpected end of expression".into()),
        }
    }

    fn name(&mut self, name: &str) -> Result<f64, String> {
        match name {
            "pi" => Ok(std::f64::consts::PI),
            "tau" => Ok(std::f64::consts::TAU),
            _ => (self.lookup)(name),
        }
    }

    fn call(&mut self, name: &str) -> Result<f64, String> {
        let mut args = vec![self.sum()?];
        while self.eat(b',') {
            args.push(self.sum()?);
        }
        if !self.eat(b')') {
            return Err(format!("missing `)` after {name}("));
        }
        let one = |f: fn(f64) -> f64| match args[..] {
            [x] => Ok(f(x)),
            _ => Err(format!("{name} takes one argument")),
        };
        match name {
            // Trigonometry is in degrees, like every angle in a document.
            "sin" => one(|x| x.to_radians().sin()),
            "cos" => one(|x| x.to_radians().cos()),
            "tan" => one(|x| x.to_radians().tan()),
            "asin" => one(|x| x.asin().to_degrees()),
            "acos" => one(|x| x.acos().to_degrees()),
            "atan" => one(|x| x.atan().to_degrees()),
            "sqrt" => one(f64::sqrt),
            "abs" => one(f64::abs),
            "floor" => one(f64::floor),
            "ceil" => one(f64::ceil),
            "round" => one(f64::round),
            "min" => Ok(args.iter().copied().fold(f64::INFINITY, f64::min)),
            "max" => Ok(args.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
            _ => Err(format!("unknown function `{name}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &str) -> Result<f64, String> {
        eval(s, &mut |n| if n == "w" { Ok(4.) } else { Err(format!("unknown `{n}`")) })
    }

    #[test]
    fn precedence_and_functions() {
        assert_eq!(run("1 + 2 * 3"), Ok(7.));
        assert_eq!(run("(1 + 2) * 3"), Ok(9.));
        assert_eq!(run("-w ^ 2"), Ok(-16.));
        assert_eq!(run("2 ^ 3 ^ 2"), Ok(512.));
        assert_eq!(run("max(w, 2, sqrt(100))"), Ok(10.));
        assert_eq!(run("1.5e2 - 1e-1"), Ok(149.9));
        assert!((run("sin(90)").unwrap() - 1.).abs() < 1e-12);
    }

    #[test]
    fn errors_are_reported() {
        assert!(run("1 +").is_err());
        assert!(run("nope").unwrap_err().contains("nope"));
        assert!(run("1 / 0").is_err());
        assert!(run("2 2").is_err());
        assert!(run("frob(1)").is_err());
    }
}
