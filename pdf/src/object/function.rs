use crate as pdf;
use crate::object::*;
use crate::error::*;

#[derive(Object, Debug)]
struct RawFunction {
    #[pdf(key="FunctionType")]
    function_type: u32,

    #[pdf(key="Domain")]
    domain: Vec<f32>,

    #[pdf(key="Range")]
    range: Option<Vec<f32>>,

    #[pdf(other)]
    other: Dictionary
}

#[derive(Object, Debug)]
struct Function2 {
    #[pdf(key="C0")]
    c0: Option<Vec<f32>>,

    #[pdf(key="C1")]
    c1: Option<Vec<f32>>,

    #[pdf(key="N")]
    exponent: f32,
}

#[derive(Debug)]
pub enum Function {
    Sampled,
    Interpolated(Vec<InterpolatedFunctionDim>),
    Stiching,
    Calculator,
    PostScript(PsFunc),
}
impl Function {
    pub fn apply(&self, x: f32, out: &mut [f32]) {
        match *self {
            Function::Interpolated(ref parts) => {
                for (f, y) in parts.iter().zip(out) {
                    *y = f.apply(x);
                }
            }
            Function::PostScript(ref func) => func.exec(x, out).unwrap(),
            _ => panic!("unimplemted function {:?}", self)
        }
    }
}
impl Object for Function {
    fn serialize<W: io::Write>(&self, _out: &mut W) -> Result<()> {
        unimplemented!()
    }
    fn from_dict(dict: Dictionary, resolve: &impl Resolve) -> Result<Self> {
        use std::f32::INFINITY;
        let raw = RawFunction::from_dict(dict, resolve)?;
        match raw.function_type {
            2 => {
                let f2 = Function2::from_dict(raw.other, resolve)?;
                let mut parts = Vec::with_capacity(raw.domain.len());
                
                let n_dim = match (raw.range.as_ref(), f2.c0.as_ref(), f2.c1.as_ref()) {
                    (Some(range), _, _) => range.len() / 2,
                    (_, Some(c0), _) => c0.len(),
                    (_, _, Some(c1)) => c1.len(),
                    _ => panic!("unknown dimensions")
                };
                let input_range = (raw.domain[0], raw.domain[1]);
                for dim in 0 .. n_dim {
                    let output_range = (
                        raw.range.as_ref().and_then(|r| r.get(2*dim).cloned()).unwrap_or(-INFINITY),
                        raw.range.as_ref().and_then(|r| r.get(2*dim+1).cloned()).unwrap_or(INFINITY)
                    );
                    let c0 = f2.c0.as_ref().and_then(|c0| c0.get(dim).cloned()).unwrap_or(0.0);
                    let c1 = f2.c1.as_ref().and_then(|c1| c1.get(dim).cloned()).unwrap_or(1.0);
                    let exponent = f2.exponent;
                    parts.push(InterpolatedFunctionDim {
                        input_range, output_range, c0, c1, exponent
                    });
                }
                Ok(Function::Interpolated(parts))
            },
            _ => {
                dbg!(raw);
                unimplemented!()
            }
        }
    }
    fn from_primitive(p: Primitive, resolve: &impl Resolve) -> Result<Self> {
        dbg!(&p);
        match p {
            Primitive::Dictionary(dict) => Self::from_dict(dict, resolve),
            Primitive::Stream(s) => {
                match s.info.get("FunctionType") {
                    Some(Primitive::Integer(4)) => {},
                    _ => panic!()
                }
                let stream = Stream::<()>::from_stream(s, resolve)?;
                let data = stream.decode()?;
                let s = std::str::from_utf8(&*data)?;
                println!("{}", s);
                let func = PsFunc::parse(s)?;
                Ok(Function::PostScript(func))
            },
            Primitive::Reference(r) => Self::from_primitive(resolve.resolve(r)?, resolve),
            _ => unimplemented!()
        }
    }
}

#[derive(Debug)]
pub struct InterpolatedFunctionDim {
    pub input_range: (f32, f32),
    pub output_range: (f32, f32),
    pub c0: f32,
    pub c1: f32,
    pub exponent: f32,
}
impl InterpolatedFunctionDim {
    pub fn apply(&self, x: f32) -> f32 {
        let y = self.c0 + x.powf(self.exponent) * (self.c1 - self.c0);
        let (y0, y1) = self.output_range;
        y.min(y1).max(y0)
    }
}

#[derive(Debug)]
pub enum PostScriptError {
    StackUnderflow,
    IncorrectStackSize
}
#[derive(Debug)]
pub struct PsFunc {
    pub ops: Vec<PsOp>
}

macro_rules! op {
    ($stack:ident; $($v:ident),* => $($e:expr),*) => ( {
        $(let $v = $stack.pop().ok_or(PostScriptError::StackUnderflow)?;)*
        $($stack.push($e);)*
    } )
}

impl PsFunc {
    pub fn exec(&self, input: f32, output: &mut [f32]) -> Result<(), PostScriptError> {
        let mut stack = Vec::with_capacity(10);
        stack.push(input);
        for &op in &self.ops {
            match op {
                PsOp::Value(v) => stack.push(v),
                PsOp::Dup => op!(stack; v => v, v),
                PsOp::Exch => op!(stack; a, b => a, b),
                PsOp::Add => op!(stack; a, b => a + b),
                PsOp::Mul => op!(stack; a, b => a * b),
                PsOp::Abs => op!(stack; a => a.abs()),
            }
        }
        output.copy_from_slice(&stack);
        Ok(())
    }
    pub fn parse(s: &str) -> Result<Self, PdfError> {
        let start = s.find("{").ok_or(PdfError::PostScriptParse)?;
        let end = s.rfind("}").ok_or(PdfError::PostScriptParse)?;

        let ops: Result<Vec<_>, _> = s[start + 1 .. end].split_ascii_whitespace().map(|p| PsOp::parse(p).ok_or(PdfError::PostScriptParse)).collect();
        Ok(PsFunc { ops: ops? })
    }
}

#[derive(Copy, Clone, Debug)]
pub enum PsOp {
    Value(f32),
    Add,
    Abs,
    Mul,
    Dup,
    Exch,
}
impl PsOp {
    pub fn parse(s: &str) -> Option<Self> {
        if let Ok(f) = s.parse() {
            Some(PsOp::Value(f))
        } else {
            Some(match s {
                "add" => PsOp::Add,
                "abs" => PsOp::Abs,
                "mul" => PsOp::Mul,
                "dup" => PsOp::Dup,
                "exch" => PsOp::Exch,
                _ => return None
            })
        }
    }
}