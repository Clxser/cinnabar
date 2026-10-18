//! Library tools retain item identity across asynchronous refreshes and discard prompts.

use super::*;
use launcher::menu::studio::library::LibraryAction;

impl Studio {
    /// The search editor is runtime-only and issues no IO until Apply.
    pub(in crate::menu) fn begin_library_search(&mut self) {
        if self.view.workspace.library.draft.is_none() {
            self.search_text
                .set_text(&self.view.workspace.library.search);
            self.publish_library_search();
        }
    }

    /// Publishes only bounded input text, without filtering on the render thread.
    pub(in crate::menu) fn publish_library_search(&mut self) {
        Arc::make_mut(&mut self.view).workspace.library.draft =
            Some(Arc::from(self.search_text.as_str()));
    }

    /// Cancelling input retains the last admitted worker query.
    pub(in crate::menu) fn cancel_library_search(&mut self) {
        Arc::make_mut(&mut self.view).workspace.library.draft = None;
        self.search_text.set_text("");
    }

    /// Returns the actual available controls for keyboard navigation.
    pub(super) fn library_focus_actions(&self) -> Vec<Action> {
        if self.view.library_loading {
            return Vec::new();
        }
        let mut actions = vec![LibraryAction::Search, LibraryAction::CycleSort];
        if self.view.workspace.library.draft.is_some() {
            actions.extend([LibraryAction::ApplySearch, LibraryAction::CancelSearch]);
        }
        if !self.view.workspace.library.search.is_empty() {
            actions.push(LibraryAction::ClearSearch);
        }
        actions.into_iter().map(Action::LibraryTools).collect()
    }

    /// Changes a query atomically with job admission; an in-flight listing cannot lose a newer query.
    pub(super) fn library_action(&mut self, action: LibraryAction) -> Result<(), String> {
        if action == LibraryAction::CancelSearch {
            self.cancel_library_search();
            return Ok(());
        }
        if self.library_job.is_some() {
            return Err("Wait for the current library search to finish.".into());
        }
        if action == LibraryAction::Search {
            self.begin_library_search();
            return Ok(());
        }
        let before = self.view.workspace.library.clone();
        let mut next = before.clone();
        match action {
            LibraryAction::ApplySearch => next.search = Arc::from(self.search_text.as_str().trim()),
            LibraryAction::ClearSearch => next.search = Arc::from(""),
            LibraryAction::CycleSort => next.sort = next.sort.next(),
            LibraryAction::Search | LibraryAction::CancelSearch => unreachable!(),
        }
        next.query().validate().map_err(|error| error.to_string())?;
        if action != LibraryAction::CycleSort {
            next.draft = None;
        }
        Arc::make_mut(&mut self.view).workspace.library = next;
        if let Err(error) = self.refresh() {
            Arc::make_mut(&mut self.view).workspace.library = before;
            return Err(error);
        }
        if action != LibraryAction::CycleSort {
            self.search_text.set_text("");
        }
        Ok(())
    }

    /// Captures the selected file before a dirty-document prompt can reorder the visible rows.
    pub(super) fn library_intent(
        &self,
        generation: u64,
        index: usize,
    ) -> Result<DiscardIntent, String> {
        if self.view.library_loading || generation != self.view.workspace.library.generation {
            return Err("The library changed. Select the item again.".into());
        }
        let entry = self
            .view
            .library
            .get(index)
            .ok_or("This library item is no longer available. Refresh the library.")?;
        Ok(DiscardIntent::Open {
            path: entry.path.clone(),
            kind: entry.kind,
            interrupted: entry.interrupted,
        })
    }

    /// The confirmed request opens the captured path, never a subsequently refreshed row index.
    pub(super) fn open_library_item(
        &mut self,
        path: PathBuf,
        kind: LibraryKind,
        interrupted: bool,
    ) -> Result<(), String> {
        if interrupted && self.view.capture.phase.busy() {
            return Err(
                "Stop and finish recording before attempting interrupted-take recovery.".into(),
            );
        }
        self.ensure_storage()?;
        self.thumbnails_cancel.cancel();
        self.open_cancel.cancel();
        self.open_cancel = Default::default();
        let request = match kind {
            LibraryKind::Project => StorageRequest::LoadProject { path },
            LibraryKind::Recording if interrupted => StorageRequest::RecoverRecording {
                path,
                cancellation: self.open_cancel.clone(),
            },
            LibraryKind::Recording => StorageRequest::OpenRecording { path },
        };
        self.open_job = Some(
            self.storage
                .as_mut()
                .expect("storage exists")
                .request(request)
                .map_err(|error| error.to_string())?,
        );
        Arc::make_mut(&mut self.view).loading = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recording_studio::storage::LibraryEntry;

    #[test]
    fn pending_open_keeps_its_identity_when_the_library_reorders() {
        let mut studio = Studio::new(PathBuf::from("unused-test-library"));
        let item = |name: &str| LibraryEntry {
            path: PathBuf::from(name),
            name: name.into(),
            kind: LibraryKind::Project,
            modified_unix_ms: 0,
            interrupted: false,
        };
        Arc::make_mut(&mut studio.view).library = vec![item("first"), item("second")].into();
        let intent = studio.library_intent(0, 0).unwrap();
        Arc::make_mut(&mut studio.view).library = vec![item("second"), item("first")].into();
        assert!(
            matches!(intent, DiscardIntent::Open { path, .. } if path == PathBuf::from("first"))
        );
    }
    #[test]
    fn painted_rows_from_an_older_listing_cannot_open_a_new_item() {
        let mut studio = Studio::new(PathBuf::from("unused-test-library"));
        Arc::make_mut(&mut studio.view).library = vec![LibraryEntry {
            path: PathBuf::from("new-item"),
            name: "New item".into(),
            kind: LibraryKind::Project,
            modified_unix_ms: 0,
            interrupted: false,
        }]
        .into();
        Arc::make_mut(&mut studio.view).workspace.library.generation = 2;
        assert!(studio.library_intent(1, 0).is_err());
        assert!(studio.library_intent(2, 0).is_ok());
    }
}
