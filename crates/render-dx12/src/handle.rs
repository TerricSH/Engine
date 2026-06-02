pub(crate) struct HandleTable<T> {
    entries: Vec<Option<(u32, T)>>,
}

impl<T> HandleTable<T> {
    pub(crate) fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub(crate) fn insert(&mut self, value: T) -> u32 {
        for (idx, slot) in self.entries.iter_mut().enumerate() {
            if slot.is_none() {
                let gen = match slot {
                    Some((g, _)) => *g + 1,
                    None => 1,
                };
                *slot = Some((gen, value));
                return idx as u32;
            }
        }
        self.entries.push(Some((1, value)));
        (self.entries.len() - 1) as u32
    }

    pub(crate) fn get(&self, index: u32) -> Option<&T> {
        self.entries
            .get(index as usize)
            .and_then(|s| s.as_ref().map(|(_, v)| v))
    }

    pub(crate) fn get_mut(&mut self, index: u32) -> Option<&mut T> {
        self.entries
            .get_mut(index as usize)
            .and_then(|s| s.as_mut().map(|(_, v)| v))
    }

    pub(crate) fn remove(&mut self, index: u32) -> Option<T> {
        self.entries
            .get_mut(index as usize)
            .and_then(|s| s.take().map(|(_, v)| v))
    }
}
