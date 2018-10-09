impl crate::oxy::Oxy {
    pub(crate) fn local_private_key(&self) -> Vec<u8> {
        self.i
            .config
            .lock()
            .local_private_key
            .as_ref()
            .unwrap()
            .clone()
    }

    pub(crate) fn remote_public_key(&self) -> Vec<u8> {
        self.i
            .config
            .lock()
            .remote_public_key
            .as_ref()
            .unwrap()
            .clone()
    }

    pub(crate) fn decrypt_outer_packet<T>(
        &self,
        packet: &[u8],
        callback: impl FnOnce(&mut [u8]) -> T,
    ) -> Result<T, ()> {
        let key = self.outer_key();
        crate::outer::decrypt_outer_packet(&key, packet, callback)
    }

    pub(crate) fn encrypt_outer_packet<T, R>(&self, interior: &[u8], callback: T) -> R
    where
        T: FnOnce(&mut [u8]) -> R,
    {
        let key = self.outer_key();
        crate::outer::encrypt_outer_packet(&key, interior, callback)
    }

    pub(crate) fn outer_key(&self) -> Vec<u8> {
        self.i.config.lock().outer_key.as_ref().unwrap().clone()
    }
}
