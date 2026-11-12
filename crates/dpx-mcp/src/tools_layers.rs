//! The layer tools (ADR-44, "Integration (wave 8)"): list, create, select, delete, show or
//! hide, rename, move and merge the layers of the active page.
//!
//! Every tool runs the engine's layer command, exactly as the layer panel does (no pixel
//! logic here): one undo step each, except `select_layer`, a setting like Go To Frame. Layer
//! numbers start at 0, the bottom layer, which is opaque; on every layer above it colour 0 is
//! transparent. The drawing tools, `fill_area` and every page command of `execute_command`
//! paint the active layer. A call that would change nothing (selecting the active layer, the
//! visibility or name a layer already has, moving a layer onto its own place) runs no command
//! and says `changed: false`.

use dpx_core::layer::MAX_NAME_CHARS;
use dpx_core::{Command, Document, IndexedImage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Spec, ToolCtx, edit, parse, read, schema};
use crate::error::{ToolError, success_result};
use crate::workspace::Workspace;
use rmcp::model::{CallToolResult, JsonObject};

/// What every layer result and `get_document_info` say about colour 0.
pub const TRANSPARENCY: &str = "layer 0 (the bottom layer) is opaque; on every layer above it \
colour 0 is transparent, so drawing with colour 0 there erases to the layers below";

// ---------------------------------------------------------------------------------------
// Arguments

/// Arguments of `create_layer`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateLayerArgs {
    /// Name of the new layer (1 to 255 characters, no control characters). Default
    /// "Layer N" with the smallest unused N.
    pub name: Option<String>,
}

/// A layer number.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerArgs {
    /// Layer number: 0 is the bottom layer (see list_layers).
    pub layer: u32,
}

/// Arguments of `set_layer_visibility`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerVisibilityArgs {
    /// Layer number: 0 is the bottom layer.
    pub layer: u32,
    /// Shown (true) or hidden (false). Hidden layers do not show and are not exported.
    pub visible: bool,
}

/// Arguments of `rename_layer`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameLayerArgs {
    /// Layer number: 0 is the bottom layer.
    pub layer: u32,
    /// The new name: 1 to 255 characters, no control characters.
    pub name: String,
}

/// Arguments of `move_layer`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveLayerArgs {
    /// The layer to move (0 = bottom).
    pub from: u32,
    /// Its new position (0 = bottom); the other layers close up.
    pub to: u32,
}

// ---------------------------------------------------------------------------------------
// The tool table

pub(super) static SPECS: &[Spec] = &[
    read(
        "list_layers",
        "List layers",
        "The layers of the active page, bottom first: index (0 = bottom), name, visible and active. The drawing tools paint the active layer; layer 0 is opaque, above it colour 0 is transparent.",
        schema::<super::NoArgs>,
    ),
    edit(
        "create_layer",
        "Create layer",
        "Add a transparent layer (colour 0 everywhere) above the active one, in every animation frame; it becomes the active layer, so the drawing tools paint it. Optional name (default \"Layer N\"). One undo step; at most 32 layers.",
        schema::<CreateLayerArgs>,
        false,
    ),
    Spec {
        idempotent: true,
        ..edit(
            "select_layer",
            "Select layer",
            "Make layer `layer` (0 = bottom) the active layer the drawing tools paint, as a click on its row in the layer panel. A setting, not an undo step.",
            schema::<LayerArgs>,
            false,
        )
    },
    edit(
        "delete_layer",
        "Delete layer",
        "Delete layer `layer` (0 = bottom) from every frame (one undo step). The last layer cannot be deleted; when the active layer goes, the layer below becomes active.",
        schema::<LayerArgs>,
        true,
    ),
    Spec {
        idempotent: true,
        ..edit(
            "set_layer_visibility",
            "Show or hide layer",
            "Show (visible=true) or hide (visible=false) layer `layer` (one undo step). Hidden layers do not show in the picture, previews or exports; they can still be painted.",
            schema::<LayerVisibilityArgs>,
            false,
        )
    },
    Spec {
        idempotent: true,
        ..edit(
            "rename_layer",
            "Rename layer",
            "Rename layer `layer` (1 to 255 characters; one undo step).",
            schema::<RenameLayerArgs>,
            false,
        )
    },
    edit(
        "move_layer",
        "Move layer",
        "Move layer `from` to position `to` (0 = bottom) in every frame; the others close up and the active layer stays the same layer (one undo step). A layer moved to the bottom becomes opaque.",
        schema::<MoveLayerArgs>,
        false,
    ),
    edit(
        "merge_layer_down",
        "Merge layer down",
        "Merge layer `layer` (1 or above) into the layer below it in every frame: its pixels other than colour 0 cover the lower layer, which keeps its name and visibility (one undo step).",
        schema::<LayerArgs>,
        false,
    ),
];

/// True for the tools of this module.
pub(super) fn handles(name: &str) -> bool {
    SPECS.iter().any(|s| s.name == name)
}

pub(super) async fn dispatch(
    ctx: &ToolCtx,
    name: &str,
    args: Option<JsonObject>,
) -> Result<CallToolResult, ToolError> {
    match name {
        "list_layers" => {
            let super::NoArgs {} = parse(args)?;
            let list = ctx.exec.run(|ws| list(ws)).await?;
            Ok(success_result(&list))
        }
        "create_layer" => {
            let a: CreateLayerArgs = parse(args)?;
            if let Some(n) = &a.name {
                check_name(n)?;
            }
            change(ctx, move |doc| {
                let at = doc.active_layer() + 1;
                let log = match &a.name {
                    Some(n) => format!("create_layer {n:?} (layer {at})"),
                    None => format!("create_layer (layer {at})"),
                };
                Ok(Step::run(Command::AddLayer { name: a.name }, at, log))
            })
            .await
        }
        "select_layer" => {
            let LayerArgs { layer } = parse(args)?;
            change(ctx, move |doc| {
                let l = index(doc, layer, "select_layer")?;
                Ok(Step::new(
                    (l != doc.active_layer()).then_some(Command::SelectLayer { layer }),
                    l,
                    format!("select_layer {l}"),
                ))
            })
            .await
        }
        "delete_layer" => {
            let LayerArgs { layer } = parse(args)?;
            change(ctx, move |doc| {
                let l = index(doc, layer, "delete_layer")?;
                if doc.layer_count() == 1 {
                    return Err(ToolError::invalid(
                        "layer 0 is the only layer and cannot be deleted: a picture keeps at least one layer",
                    ));
                }
                let name = &doc.layers()[l].name;
                let log = format!("delete_layer {l} {name:?}");
                Ok(Step::run(Command::DeleteLayer { layer }, l, log))
            })
            .await
        }
        "set_layer_visibility" => {
            let LayerVisibilityArgs { layer, visible } = parse(args)?;
            change(ctx, move |doc| {
                let l = index(doc, layer, "set_layer_visibility")?;
                let same = doc.layers()[l].visible == visible;
                Ok(Step::new(
                    (!same).then_some(Command::SetLayerVisible { layer, visible }),
                    l,
                    format!(
                        "set_layer_visibility {l} {}",
                        if visible { "shown" } else { "hidden" }
                    ),
                ))
            })
            .await
        }
        "rename_layer" => {
            let RenameLayerArgs { layer, name } = parse(args)?;
            check_name(&name)?;
            change(ctx, move |doc| {
                let l = index(doc, layer, "rename_layer")?;
                let same = doc.layers()[l].name == name;
                let log = format!("rename_layer {l} {name:?}");
                Ok(Step::new(
                    (!same).then_some(Command::RenameLayer { layer, name }),
                    l,
                    log,
                ))
            })
            .await
        }
        "move_layer" => {
            let MoveLayerArgs { from, to } = parse(args)?;
            change(ctx, move |doc| {
                let f = index(doc, from, "move_layer from")?;
                let t = index(doc, to, "move_layer to")?;
                Ok(Step::new(
                    (f != t).then_some(Command::MoveLayer { from, to }),
                    t,
                    format!("move_layer {f} to {t}"),
                ))
            })
            .await
        }
        "merge_layer_down" => {
            let LayerArgs { layer } = parse(args)?;
            change(ctx, move |doc| {
                let l = index(doc, layer, "merge_layer_down")?;
                if l == 0 {
                    return Err(ToolError::invalid(
                        "layer 0 is the bottom layer: there is no layer below to merge it into",
                    ));
                }
                let log = format!("merge_layer_down {l}");
                Ok(Step::run(Command::MergeLayerDown { layer }, l - 1, log))
            })
            .await
        }
        other => Err(ToolError::internal(format!("no layer tool {other}"))),
    }
}

// ---------------------------------------------------------------------------------------
// Descriptions

/// One layer as the tools describe it.
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct LayerEntry {
    /// Layer number: 0 is the bottom layer.
    pub index: usize,
    pub name: String,
    pub visible: bool,
    /// The drawing tools paint this layer.
    pub active: bool,
}

/// The layer stack of `doc`, bottom first.
pub fn entries(doc: &Document) -> Vec<LayerEntry> {
    let active = doc.active_layer();
    doc.layers()
        .iter()
        .enumerate()
        .map(|(index, l)| LayerEntry {
            index,
            name: l.name.clone(),
            visible: l.visible,
            active: index == active,
        })
        .collect()
}

/// The result of `list_layers`.
#[derive(Serialize)]
struct LayerList {
    document_id: &'static str,
    revision: u64,
    layer_count: usize,
    active_layer: usize,
    /// Bottom first.
    layers: Vec<LayerEntry>,
    transparency: &'static str,
}

fn list(ws: &dyn Workspace) -> LayerList {
    let doc = ws.engine().document();
    LayerList {
        document_id: ws.shown().id(),
        revision: doc.revision(),
        layer_count: doc.layer_count(),
        active_layer: doc.active_layer(),
        layers: entries(doc),
        transparency: TRANSPARENCY,
    }
}

/// The result of a changing layer tool.
#[derive(Serialize)]
struct LayerChange {
    document_id: &'static str,
    revision: u64,
    /// False when the call asked for what already was (no command ran).
    changed: bool,
    /// The call made an undo step (every layer change but `select_layer`).
    undoable: bool,
    can_undo: bool,
    can_redo: bool,
    /// The layer the call was about, as numbered after it (`create_layer`: the new layer;
    /// `move_layer`: its new position; `delete_layer`: the number it had; `merge_layer_down`:
    /// the layer it went into).
    layer: usize,
    layer_count: usize,
    active_layer: usize,
    /// Bottom first.
    layers: Vec<LayerEntry>,
}

// ---------------------------------------------------------------------------------------
// Running

/// What a layer tool runs: the command (none if it would change nothing), the layer the
/// result names and the activity log line.
struct Step {
    command: Option<Command>,
    layer: usize,
    log: String,
}

impl Step {
    fn new(command: Option<Command>, layer: usize, log: String) -> Self {
        Self {
            command,
            layer,
            log,
        }
    }

    fn run(command: Command, layer: usize, log: String) -> Self {
        Self::new(Some(command), layer, log)
    }
}

/// Plans the step on the shown page's document and runs its command through the workspace
/// (the engine, as the layer panel does), in one job.
async fn change(
    ctx: &ToolCtx,
    plan: impl FnOnce(&Document) -> Result<Step, ToolError> + Send + 'static,
) -> Result<CallToolResult, ToolError> {
    let result = ctx
        .exec
        .run(move |ws| -> Result<LayerChange, ToolError> {
            let step = plan(ws.engine().document())?;
            let undoable = !matches!(step.command, None | Some(Command::SelectLayer { .. }));
            let changed = step.command.is_some();
            if let Some(command) = step.command {
                ws.execute(command.into())?;
            }
            ws.log(&step.log);
            let engine = ws.engine();
            let doc = engine.document();
            Ok(LayerChange {
                document_id: ws.shown().id(),
                revision: doc.revision(),
                changed,
                undoable,
                can_undo: engine.can_undo(),
                can_redo: engine.can_redo(),
                layer: step.layer,
                layer_count: doc.layer_count(),
                active_layer: doc.active_layer(),
                layers: entries(doc),
            })
        })
        .await??;
    Ok(success_result(&result))
}

/// Layer `layer` of `doc`, or `LAYER_NOT_FOUND` naming the layers there are.
fn index(doc: &Document, layer: u32, what: &str) -> Result<usize, ToolError> {
    let n = doc.layer_count();
    let l = layer as usize;
    if l < n {
        return Ok(l);
    }
    let there = if n == 1 {
        "the picture has 1 layer, layer 0".to_string()
    } else {
        format!("the picture has {n} layers, 0 (bottom) to {} (top)", n - 1)
    };
    Err(ToolError::new(
        "LAYER_NOT_FOUND",
        format!("{what}: there is no layer {layer}; {there} (see list_layers)"),
    ))
}

/// A layer name: 1 to [`MAX_NAME_CHARS`] characters, not only spaces, no control characters.
fn check_name(name: &str) -> Result<(), ToolError> {
    let n = name.chars().count();
    if name.trim().is_empty() {
        return Err(ToolError::invalid("name: a layer name cannot be empty"));
    }
    if n > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(ToolError::invalid(format!(
            "name: at most {MAX_NAME_CHARS} characters, no control characters ({n} given)"
        )));
    }
    Ok(())
}

/// Layer `layer` of the current frame alone, for `get_preview`: its picture, the colour the
/// PNG leaves transparent (colour 0 above the bottom layer) and what the result says about
/// it.
pub(super) fn preview_picture(
    doc: &Document,
    layer: u32,
) -> Result<(&IndexedImage, Option<u8>, String), ToolError> {
    let l = index(doc, layer, "get_preview layer")?;
    let picture = doc.layer(l).expect("an existing layer");
    let info = &doc.layers()[l];
    let shown = if info.visible {
        "shown in the picture"
    } else {
        "hidden in the picture"
    };
    Ok(if l == 0 {
        let c0 = doc.palette().colors()[0];
        (
            picture,
            None,
            format!(
                "layer 0 {:?} alone ({shown}); it is the bottom layer, so colour 0 is opaque and shows as palette colour 0 ({c0})",
                info.name
            ),
        )
    } else {
        (
            picture,
            Some(0),
            format!(
                "layer {l} {:?} alone ({shown}); colour 0 is transparent on this layer and left transparent (alpha 0) in the PNG",
                info.name
            ),
        )
    })
}
