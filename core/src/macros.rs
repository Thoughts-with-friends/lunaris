#[macro_export]
macro_rules! impl_savestate_bitflags {
    ($ty:ty) => {
        impl emu_utils::Storable for $ty {
            fn store<S: emu_utils::WriteSavestate>(
                &mut self,
                save: &mut S,
            ) -> Result<(), S::Error> {
                let Self { bits } = self;
                save.start_struct()?;
                save.start_field(b"bits")?;
                save.store(bits)?;
                save.end_struct()?;
                Ok(())
            }
        }

        impl emu_utils::Loadable for $ty {
            fn load<S: emu_utils::ReadSavestate>(save: &mut S) -> Result<Self, S::Error> {
                save.start_struct()?;
                save.start_field(b"bits")?;
                let bits = save.load()?;
                save.end_struct()?;
                Ok(Self { bits })
            }
        }

        impl emu_utils::LoadableInPlace for $ty {
            fn load_in_place<S: emu_utils::ReadSavestate>(
                &mut self,
                save: &mut S,
            ) -> Result<(), S::Error> {
                save.start_struct()?;
                save.start_field(b"bits")?;
                save.load_into(&mut self.bits)?;
                save.end_struct()?;
                Ok(())
            }
        }
    };
}
