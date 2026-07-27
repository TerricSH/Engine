use engine_scene::{World, WorldSlot};
use engine_serialize::{Diagnostic, DiagnosticSeverity};

pub(crate) fn apply_script_ui_command(
    world_slot: &WorldSlot,
    requested_by: &str,
    command: engine_script::GameplayUiCommand,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Err(reason) = command.validate() {
        diagnostics.push(entity_diagnostic(
            "SCRIPT_UI_COMMAND_INVALID",
            requested_by,
            requested_by,
            format!("script entity '{requested_by}' produced an invalid UI command: {reason}"),
        ));
        return;
    }

    let canvas_id = command_canvas_id(&command).to_owned();
    let applied = world_slot.with_world_mut(|world| apply_to_world(world, command));
    match applied {
        Some(Ok(())) => {}
        Some(Err(reason)) => diagnostics.push(entity_diagnostic(
            "SCRIPT_UI_COMMAND_FAILED",
            &canvas_id,
            requested_by,
            format!(
                "script entity '{requested_by}' could not mutate canvas '{canvas_id}': {reason}"
            ),
        )),
        None => diagnostics.push(entity_diagnostic(
            "SCRIPT_WORLD_MISSING",
            requested_by,
            requested_by,
            format!(
                "script entity '{requested_by}' could not mutate canvas '{canvas_id}' because no World is active"
            ),
        )),
    }
}

fn command_canvas_id(command: &engine_script::GameplayUiCommand) -> &str {
    match command {
        engine_script::GameplayUiCommand::CreateCanvas { canvas_id, .. }
        | engine_script::GameplayUiCommand::RemoveCanvas { canvas_id }
        | engine_script::GameplayUiCommand::ResizeCanvas { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetCanvasScaleMode { canvas_id, .. }
        | engine_script::GameplayUiCommand::ClearCanvas { canvas_id }
        | engine_script::GameplayUiCommand::AddElement { canvas_id, .. }
        | engine_script::GameplayUiCommand::RemoveElement { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetElementEnabled { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetText { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetToggleValue { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetCheckboxValue { canvas_id, .. }
        | engine_script::GameplayUiCommand::SetSliderValue { canvas_id, .. } => canvas_id,
    }
}

fn apply_to_world(
    world: &mut World,
    command: engine_script::GameplayUiCommand,
) -> Result<(), String> {
    use engine_script::GameplayUiCommand as Command;

    match command {
        Command::CreateCanvas {
            canvas_id,
            width,
            height,
        } => {
            if world.entity_by_persistent_id(&canvas_id).is_some() {
                return Err(format!(
                    "canvas '{canvas_id}' cannot be created because that persistent entity already exists"
                ));
            }
            let entity = world
                .create_persistent_entity(canvas_id.clone())
                .map_err(|error| format!("create canvas entity '{canvas_id}': {error}"))?;
            world.add_component(entity, engine_ui::Canvas::new(width, height));
            Ok(())
        }
        Command::RemoveCanvas { canvas_id } => {
            let entity = world
                .entity_by_persistent_id(&canvas_id)
                .ok_or_else(|| format!("canvas '{canvas_id}' does not exist"))?;
            world
                .remove_component::<engine_ui::Canvas>(entity)
                .map(|_| ())
                .ok_or_else(|| format!("entity '{canvas_id}' has no UI Canvas"))
        }
        Command::ResizeCanvas {
            canvas_id,
            width,
            height,
        } => {
            canvas_mut(world, &canvas_id)?.resize(width, height);
            Ok(())
        }
        Command::SetCanvasScaleMode {
            canvas_id,
            scale_mode,
        } => {
            canvas_mut(world, &canvas_id)?.scale_mode = match scale_mode {
                engine_script::GameplayUiScaleMode::Fixed => engine_ui::ScaleMode::Fixed,
                engine_script::GameplayUiScaleMode::FitWidth => engine_ui::ScaleMode::FitWidth,
                engine_script::GameplayUiScaleMode::FitHeight => engine_ui::ScaleMode::FitHeight,
            };
            Ok(())
        }
        Command::ClearCanvas { canvas_id } => {
            canvas_mut(world, &canvas_id)?.clear();
            Ok(())
        }
        Command::AddElement {
            canvas_id,
            element_id,
            element,
        } => canvas_mut(world, &canvas_id)?
            .insert_element(engine_ui::ElementId(element_id), runtime_element(element))
            .map(|_| ())
            .map_err(|error| format!("canvas '{canvas_id}': {error}")),
        Command::RemoveElement {
            canvas_id,
            element_id,
        } => canvas_mut(world, &canvas_id)?
            .remove_element(engine_ui::ElementId(element_id))
            .then_some(())
            .ok_or_else(|| format!("canvas '{canvas_id}' has no element with id {element_id}")),
        Command::SetElementEnabled {
            canvas_id,
            element_id,
            enabled,
        } => {
            element_mut(world, &canvas_id, element_id)?.enabled = enabled;
            Ok(())
        }
        Command::SetText {
            canvas_id,
            element_id,
            text,
        } => match &mut element_mut(world, &canvas_id, element_id)?.kind {
            engine_ui::UiElementKind::Text { content, .. } => {
                *content = text;
                Ok(())
            }
            _ => Err(format!(
                "canvas '{canvas_id}' element {element_id} is not a Text element"
            )),
        },
        Command::SetToggleValue {
            canvas_id,
            element_id,
            is_on,
        } => match &mut element_mut(world, &canvas_id, element_id)?.kind {
            engine_ui::UiElementKind::Toggle { is_on: current, .. } => {
                *current = is_on;
                Ok(())
            }
            _ => Err(format!(
                "canvas '{canvas_id}' element {element_id} is not a Toggle element"
            )),
        },
        Command::SetCheckboxValue {
            canvas_id,
            element_id,
            checked,
        } => match &mut element_mut(world, &canvas_id, element_id)?.kind {
            engine_ui::UiElementKind::Checkbox {
                checked: current, ..
            } => {
                *current = checked;
                Ok(())
            }
            _ => Err(format!(
                "canvas '{canvas_id}' element {element_id} is not a Checkbox element"
            )),
        },
        Command::SetSliderValue {
            canvas_id,
            element_id,
            value,
        } => match &mut element_mut(world, &canvas_id, element_id)?.kind {
            engine_ui::UiElementKind::Slider {
                value: current,
                min,
                max,
                ..
            } if value >= *min && value <= *max => {
                *current = value;
                Ok(())
            }
            engine_ui::UiElementKind::Slider { min, max, .. } => Err(format!(
                "canvas '{canvas_id}' slider {element_id} value {value} is outside [{min}, {max}]"
            )),
            _ => Err(format!(
                "canvas '{canvas_id}' element {element_id} is not a Slider element"
            )),
        },
    }
}

fn canvas_mut<'a>(
    world: &'a mut World,
    canvas_id: &str,
) -> Result<&'a mut engine_ui::Canvas, String> {
    let entity = world
        .entity_by_persistent_id(canvas_id)
        .ok_or_else(|| format!("canvas '{canvas_id}' does not exist"))?;
    world
        .get_mut::<engine_ui::Canvas>(entity)
        .ok_or_else(|| format!("entity '{canvas_id}' has no UI Canvas"))
}

fn element_mut<'a>(
    world: &'a mut World,
    canvas_id: &str,
    element_id: u32,
) -> Result<&'a mut engine_ui::UiElement, String> {
    canvas_mut(world, canvas_id)?
        .get_element_mut(engine_ui::ElementId(element_id))
        .ok_or_else(|| format!("canvas '{canvas_id}' has no element with id {element_id}"))
}

fn runtime_element(element: engine_script::GameplayUiElement) -> engine_ui::UiElement {
    use engine_script::GameplayUiElement as WireElement;
    use engine_ui::UiElementKind;

    let color = |value: engine_script::GameplayUiColor| {
        engine_ui::Color::new(value.r, value.g, value.b, value.a)
    };
    let layout = |value: engine_script::GameplayUiLayout| {
        engine_ui::Layout::new(
            glam::Vec2::from_array(value.anchor_min),
            glam::Vec2::from_array(value.anchor_max),
            glam::Vec2::from_array(value.offset_min),
            glam::Vec2::from_array(value.offset_max),
        )
    };

    let (kind, layout, z_order) = match element {
        WireElement::Panel {
            layout: value,
            color: tint,
            z_order,
        } => (
            UiElementKind::Panel { color: color(tint) },
            layout(value),
            z_order,
        ),
        WireElement::Image {
            layout: value,
            texture_id,
            color: tint,
            z_order,
        } => (
            UiElementKind::Image {
                texture_id,
                color: color(tint),
            },
            layout(value),
            z_order,
        ),
        WireElement::Text {
            layout: value,
            text,
            font_size,
            color: tint,
            z_order,
        } => (
            UiElementKind::Text {
                content: text,
                font_size,
                color: color(tint),
            },
            layout(value),
            z_order,
        ),
        WireElement::Button {
            layout: value,
            label,
            normal_color,
            hover_color,
            pressed_color,
            callback_id,
            z_order,
        } => (
            UiElementKind::Button {
                label,
                normal_color: color(normal_color),
                hover_color: color(hover_color),
                pressed_color: color(pressed_color),
                callback_id,
            },
            layout(value),
            z_order,
        ),
        WireElement::Toggle {
            layout: value,
            label,
            is_on,
            color_on,
            color_off,
            callback_id,
            z_order,
        } => (
            UiElementKind::Toggle {
                label,
                is_on,
                color_on: color(color_on),
                color_off: color(color_off),
                callback_id,
            },
            layout(value),
            z_order,
        ),
        WireElement::Checkbox {
            layout: value,
            label,
            checked,
            color: tint,
            callback_id,
            z_order,
        } => (
            UiElementKind::Checkbox {
                label,
                checked,
                color: color(tint),
                callback_id,
            },
            layout(value),
            z_order,
        ),
        WireElement::Slider {
            layout: value,
            label,
            value: slider_value,
            min,
            max,
            callback_id,
            z_order,
        } => (
            UiElementKind::Slider {
                label,
                value: slider_value,
                min,
                max,
                callback_id,
            },
            layout(value),
            z_order,
        ),
        WireElement::ScrollView {
            layout: value,
            content_width,
            content_height,
            color: tint,
            z_order,
        } => (
            UiElementKind::ScrollView {
                scroll_x: 0.0,
                scroll_y: 0.0,
                content_width,
                content_height,
                color: color(tint),
            },
            layout(value),
            z_order,
        ),
    };
    engine_ui::UiElement::new(kind, layout).with_z_order(z_order)
}

fn entity_diagnostic(code: &str, entity: &str, _requested_by: &str, message: String) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(code, DiagnosticSeverity::Error, "script", message);
    diagnostic.entity = Some(entity.to_owned());
    diagnostic
}
