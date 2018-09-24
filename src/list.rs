use std::cell::RefCell;
use std::rc::Rc;

use mio::Token;

use sources::EventDispatcher;

pub(crate) struct SourceList<Data> {
    sources: Vec<Option<Rc<RefCell<EventDispatcher<Data>>>>>,
}

impl<Data> SourceList<Data> {
    pub(crate) fn new() -> SourceList<Data> {
        SourceList {
            sources: Vec::new(),
        }
    }

    pub(crate) fn get_dispatcher(
        &self,
        token: Token,
    ) -> Option<Rc<RefCell<EventDispatcher<Data>>>> {
        match self.sources.get(token.0) {
            Some(&Some(ref dispatcher)) => Some(dispatcher.clone()),
            _ => None,
        }
    }

    pub(crate) fn add_source(&mut self, dispatcher: Rc<RefCell<EventDispatcher<Data>>>) -> Token {
        let free_id = self.sources.iter().position(Option::is_none);
        if let Some(id) = free_id {
            self.sources[id] = Some(dispatcher);
            Token(id)
        } else {
            self.sources.push(Some(dispatcher));
            Token(self.sources.len() - 1)
        }
    }

    // this method returns the removed dispatcher to ensure it is not dropped
    // while the refcell containing the list is borrowed, as dropping a dispatcher
    // can trigger the removal of an other source
    pub(crate) fn del_source(
        &mut self,
        token: Token,
    ) -> Option<Rc<RefCell<EventDispatcher<Data>>>> {
        ::std::mem::replace(&mut self.sources[token.0], None)
    }
}

pub(crate) trait ErasedList {
    // this returs a value for the same reason as above, but we must erase its type
    // due to the `Data` parameter, hence Box<Any>
    fn del_source(&mut self, token: Token) -> Box<::std::any::Any>;
}

impl<Data: 'static> ErasedList for SourceList<Data> {
    fn del_source(&mut self, token: Token) -> Box<::std::any::Any> {
        Box::new(self.del_source(token))
    }
}
