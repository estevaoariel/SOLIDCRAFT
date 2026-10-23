//! Which workspace the window shows, and switching between them. Design is the modelling
//! workspace with its toolbar tabs; Render and Simulation each have one toolbar tab of their
//! own, and that tab is what records the workspace (`UiState::tab`), so `ui.set {tab}` and the
//! saved window state switch workspaces exactly as the menu does.

use crate::SolveApp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workspace {
    Design,
    Render,
    Simulation,
}

impl Workspace {
    pub const ALL: [Workspace; 3] = [Workspace::Design, Workspace::Render, Workspace::Simulation];

    /// The id the workspace menu, the control channel and MCP use.
    pub fn id(self) -> &'static str {
        match self {
            Workspace::Design => "design",
            Workspace::Render => "render",
            Workspace::Simulation => "simulation",
        }
    }

    pub fn from_id(id: &str) -> Option<Workspace> {
        Workspace::ALL.into_iter().find(|w| w.id() == id)
    }

    /// The workspace a toolbar tab belongs to.
    pub fn of_tab(tab: &str) -> Workspace {
        match tab {
            "RENDER" => Workspace::Render,
            "SIMULATION" => Workspace::Simulation,
            _ => Workspace::Design,
        }
    }

    /// The tab the workspace opens on.
    pub fn tab(self) -> &'static str {
        match self {
            Workspace::Design => "MODEL",
            Workspace::Render => "RENDER",
            Workspace::Simulation => "SIMULATION",
        }
    }

    /// The English menu label (translated through the label adapter).
    pub fn label(self) -> &'static str {
        match self {
            Workspace::Design => "Design",
            Workspace::Render => "Render",
            Workspace::Simulation => "Simulation",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Workspace::Design => "ws_design",
            Workspace::Render => "ws_render",
            Workspace::Simulation => "ws_simulation",
        }
    }
}

pub fn current(app: &SolveApp) -> Workspace {
    Workspace::of_tab(&app.ui.tab)
}

/// Leaving Design waits until nothing is half done: no sketch, tool, dialog or file load.
pub fn can_leave_design(app: &SolveApp) -> bool {
    app.session.active_sketch.is_none() && app.tool.is_none() && app.dialog.is_none() && !app.file_load.busy()
}

/// Switch to `to`. Choosing Design always lands on its SOLID tab; choosing Render again
/// refreshes its scene; choosing the Simulation workspace while in it changes nothing.
pub fn enter(app: &mut SolveApp, to: Workspace) -> Result<(), String> {
    let from = current(app);
    match to {
        Workspace::Design => {
            crate::simulation_workspace::close(app);
            crate::render_workspace::close(app);
            app.ui.tab = to.tab().into();
        }
        Workspace::Render => {
            if !can_leave_design(app) {
                return Err("finish the current modelling operation before entering Render".into());
            }
            crate::render_workspace::open(app)?;
        }
        Workspace::Simulation if from == Workspace::Simulation => return Ok(()),
        Workspace::Simulation => {
            if !can_leave_design(app) {
                return Err("finish the current modelling operation before entering Simulation".into());
            }
            crate::render_workspace::close(app);
            app.ui.tab = to.tab().into();
        }
    }
    app.cancel_view_animation();
    Ok(())
}

#[cfg(test)]
#[path = "workspace_switch_tests.rs"]
mod tests;
