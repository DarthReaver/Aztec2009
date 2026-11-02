use super::graph::NodeId;

/// Address of a plug (an attribute on a node).
#[derive(Clone, Debug, PartialEq)]
pub struct PlugRef {
    pub node: NodeId,
    pub attr: String,
}

/// A single scalar attribute with dependency graph bookkeeping.
///
/// * `value`  is the current (possibly cached) evaluated value.
/// * `rest`   is the user authored value that animation layers add to.
/// * `dirty`  marks that an upstream source changed and `value` must be re-pulled lazily.
/// * `source` is the single incoming connection, `dests` the outgoing ones.
#[derive(Clone, Debug)]
pub struct Plug {
    pub name: String,
    pub value: f32,
    pub rest: f32,
    pub dirty: bool,
    pub source: Option<PlugRef>,
    pub dests: Vec<PlugRef>,
    pub custom: bool,
    pub readonly: bool,
    /// Shown in the Channel Box and keyable (Channel Control moves attributes in and out).
    pub keyable: bool,
    /// A locked attribute keeps its value: setAttr and the manipulators refuse to change it.
    pub locked: bool,
    /// Values are clamped to this range when set (Add Attribute min / max).
    pub min: Option<f32>,
    pub max: Option<f32>,
    /// 0 float, 1 integer, 2 boolean (custom attributes).
    pub dtype: u8,
}

/// Attributes that exist for the node's own bookkeeping and are not in the Channel Box by default.
pub fn hidden_by_default(name: &str) -> bool {
    name.starts_with("rotatePivot") || name.starts_with("scalePivot")
}

impl Plug {
    pub fn new(name: &str, v: f32) -> Plug {
        Plug {
            name: name.to_string(),
            value: v,
            rest: v,
            dirty: false,
            source: None,
            dests: Vec::new(),
            custom: false,
            readonly: false,
            keyable: !hidden_by_default(name),
            locked: false,
            min: None,
            max: None,
            dtype: 0,
        }
    }
}

/// Attributes that affect a transform matrix.
pub fn is_transform_attr(name: &str) -> bool {
    matches!(
        name,
        "translateX"
            | "inheritsTransform"
            | "jointOrientX"
            | "jointOrientY"
            | "jointOrientZ"
            | "translateY"
            | "translateZ"
            | "rotateX"
            | "rotateY"
            | "rotateZ"
            | "scaleX"
            | "scaleY"
            | "scaleZ"
            | "rotatePivotX"
            | "rotatePivotY"
            | "rotatePivotZ"
            | "scalePivotX"
            | "scalePivotY"
            | "scalePivotZ"
            | "rotatePivotTranslateX"
            | "rotatePivotTranslateY"
            | "rotatePivotTranslateZ"
    )
}

/// Maya style short names for attributes.
pub fn canonical_attr(name: &str) -> String {
    let n = match name {
        "tx" => "translateX",
        "ty" => "translateY",
        "tz" => "translateZ",
        "rx" => "rotateX",
        "ry" => "rotateY",
        "rz" => "rotateZ",
        "sx" => "scaleX",
        "sy" => "scaleY",
        "sz" => "scaleZ",
        "v" => "visibility",
        other => other,
    };
    n.to_string()
}
