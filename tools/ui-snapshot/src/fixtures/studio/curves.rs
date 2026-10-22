//! Anonymous scalar animation exposes production graph controls without account or take data.

use super::*;
use launcher::menu::studio::graph::{GraphPlot, GraphPublication};
use recording_studio::{
    camera::CameraChannel,
    edit::Editor,
    effect::Easing,
    graph::{self, GraphChannel, GraphCurve, GraphKey},
};

pub(in crate::fixtures) fn graph() -> Fixture {
    let Fixture::Menu(mut view) = super::path() else {
        unreachable!()
    };
    let studio = Arc::make_mut(&mut view.studio);
    let project = studio.project.as_ref().unwrap();
    let item = project.shots[0].id;
    let channel = GraphChannel::Camera(CameraChannel::Y);
    let base = CameraPose::default();
    let mut editor = Editor::new(project.as_ref().clone()).unwrap();
    for (at, value) in [
        (0, 66.0),
        (7_500_000, 69.0),
        (15_000_000, 67.0),
        (30_000_000, 70.0),
    ] {
        let curve =
            GraphCurve::new_reusing(Arc::clone(editor.project()), item, channel, base, None)
                .unwrap();
        editor
            .apply(
                "Curve fixture",
                &[
                    graph::set_key_on(&curve, editor.project(), None, MediaTime(at), value)
                        .unwrap(),
                ],
            )
            .unwrap();
    }
    let key = GraphKey {
        item,
        channel,
        at: MediaTime(15_000_000),
    };
    let curve =
        GraphCurve::new_reusing(Arc::clone(editor.project()), item, channel, base, None).unwrap();
    editor
        .apply(
            "Curve handles",
            &[graph::set_easing_on(
                &curve,
                editor.project(),
                key,
                Easing::Bezier {
                    outgoing: [18000, 0],
                    incoming: [55000, 65535],
                },
            )
            .unwrap()],
        )
        .unwrap();
    let project = Arc::clone(editor.project());
    let curve =
        Arc::new(GraphCurve::new_reusing(Arc::clone(&project), item, channel, base, None).unwrap());
    let window = [MediaTime::ZERO, MediaTime(30_000_000)];
    let points = (0..=512)
        .map(|i| {
            let at = MediaTime(i * 30_000_000 / 512);
            [at.0 as f64, curve.sample(at)]
        })
        .collect::<Vec<_>>();
    studio.selected = Some(item);
    studio.position = key.at;
    studio.visible_start = window[0];
    studio.visible_duration = window[1];
    studio.workspace.media_hidden = true;
    studio.workspace.inspector_hidden = true;
    studio.project = Some(Arc::clone(&project));
    studio.camera_workspace.graph = launcher::menu::studio::graph::GraphWorkspace {
        open: true,
        channel: Some(channel),
        selected: Some(key),
        range: Some([64.0, 72.0]),
        plot: Some(GraphPlot {
            curve: Arc::clone(&curve),
            window,
            points: points.into(),
        }),
        publication: Some(GraphPublication { project, curve }),
        ..Default::default()
    };
    Fixture::Menu(view)
}
