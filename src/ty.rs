//! 类型系统：数据宽类型（Bit / Bits<N>）与资源类型（Gate / Depth / Cycle）。
//!
//! 对应白皮书 v2.2 §5。类型不仅描述数据，还描述资源上界：
//! 编译器在编译期验证实际门数/深度/周期不超过声明上界。

use crate::ast::{ResourceBound, Width};

/// 类型化的值：携带位宽。资源上界挂在声明上，不做运行时值的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ty {
    pub w: Width,
}

impl Ty {
    pub fn bit() -> Self {
        Ty { w: Width::Bit }
    }
    pub fn bits(n: u32) -> Self {
        Ty { w: Width::Bits(n) }
    }
    pub fn width(&self) -> u32 {
        self.w.bits()
    }
}

/// 门级环境角色（用于校验语义）：组合函数中禁止 LATCH。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    Combinational,
    Sequential,
}

/// 资源预算累计器：编译期间增量计算静态资源。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResourceBudget {
    pub gates: u32,
    pub depth: u32,
    pub cycles: u32,
    pub latches: u32,
}

impl ResourceBudget {
    /// 检查所有声明上界。返回违规列表（空 = 通过）。
    pub fn check_bounds(&self, decls: &[ResourceBound]) -> Vec<String> {
        let mut errs = Vec::new();
        for b in decls {
            match b {
                ResourceBound::Gates(g) => {
                    if self.gates > *g {
                        errs.push(format!(
                            "资源上界违规: gates = {} > Gates<{}> 声明",
                            self.gates, g
                        ));
                    }
                }
                ResourceBound::Depth(d) => {
                    if self.depth > *d {
                        errs.push(format!(
                            "资源上界违规: depth = {} > Depth<{}> 声明",
                            self.depth, d
                        ));
                    }
                }
                ResourceBound::Cycles(c) => {
                    if self.cycles > *c {
                        errs.push(format!(
                            "资源上界违规: cycles = {} > Cycles<{}> 声明",
                            self.cycles, c
                        ));
                    }
                }
            }
        }
        errs
    }

    pub fn summary(&self) -> String {
        format!(
            "gates={} depth={} cycles={} latches={}",
            self.gates, self.depth, self.cycles, self.latches
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_check_bounds() {
        let b = ResourceBudget { gates: 10, depth: 4, cycles: 1, latches: 0 };
        assert!(b.check_bounds(&[]).is_empty());
        assert!(b.check_bounds(&[ResourceBound::Gates(10)]).is_empty());
        assert_eq!(b.check_bounds(&[ResourceBound::Gates(9)]).len(), 1);
        assert!(b.check_bounds(&[
            ResourceBound::Gates(10),
            ResourceBound::Depth(4),
            ResourceBound::Cycles(1)
        ]).is_empty());
    }
}