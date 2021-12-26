use std::rc::Rc;

use crate::sources::EventDispatcher;

pub(crate) struct SourceList<'sl, Data> {
    sources: Vec<Option<Rc<dyn EventDispatcher<Data> + 'sl>>>,
}

impl<'sl, Data> SourceList<'sl, Data> {
    pub(crate) fn new() -> Self {
        SourceList {
            sources: Vec::new(),
        }
    }

    pub(crate) fn contains(&self, id: u32) -> bool {
        self.sources
            .get(id as usize)
            .map(Option::is_some)
            .unwrap_or(false)
    }

    pub(crate) fn get_dispatcher(&self, id: u32) -> Option<Rc<dyn EventDispatcher<Data> + 'sl>> {
        match self.sources.get(id as usize) {
            Some(&Some(ref dispatcher)) => Some(dispatcher.clone()),
            _ => None,
        }
    }

    pub(crate) fn add_source(&mut self, dispatcher: Rc<dyn EventDispatcher<Data> + 'sl>) -> u32 {
        let free_id = self.sources.iter().position(Option::is_none);
        if let Some(id) = free_id {
            self.sources[id] = Some(dispatcher);
            id as u32
        } else {
            self.sources.push(Some(dispatcher));
            (self.sources.len() - 1) as u32
        }
    }

    // this method returns the removed dispatcher to ensure it is not dropped
    // while the refcell containing the list is borrowed, as dropping a dispatcher
    // can trigger the removal of an other source
    pub(crate) fn del_source(&mut self, id: u32) -> Option<Rc<dyn EventDispatcher<Data> + 'sl>> {
        ::std::mem::replace(&mut self.sources[id as usize], None)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Rc<dyn EventDispatcher<Data> + 'sl>> {
        self.sources.iter().flat_map(|o| o.as_ref())
    }
}
