//! Adapter for the original call bridges. Keep their stack offsets and flag
//! behavior while leaving all patch allocation and installation to Modloader.

use patchy_host::{CallBridge, FlagPolicy, KeepReturn, Result, StackAlignment};

pub struct PatchSession(patchy_host::PatchSession);

impl PatchSession {
    pub fn new() -> Result<Self> {
        patchy_host::PatchSession::new().map(Self)
    }

    pub unsafe fn detour_trampoline(
        &mut self,
        address: usize,
        size: usize,
        trampoline: Trampoline,
    ) -> Result<patchy_host::Patch> {
        self.0
            .detour_trampoline(address, size, |_| Ok(trampoline.0))
    }

    pub unsafe fn patch_call(
        &mut self,
        address: usize,
        callback: *const (),
        size: usize,
        replay: bool,
        result: ReturnType,
    ) -> Result<patchy_host::Patch> {
        self.0.detour_trampoline(address, size, |site| {
            let mut code = Trampoline::new();
            if replay {
                site.replay_verbatim(&mut code.0);
            }
            code.preserved_call(callback, &[], &[], result);
            site.resume(&mut code.0);
            Ok(code.0)
        })
    }

    pub unsafe fn overwrite(&mut self, address: usize, bytes: &[u8]) -> Result<patchy_host::Patch> {
        self.0.overwrite(address, bytes)
    }

    pub unsafe fn install_permanently(self) -> Result {
        self.0.install_permanently()
    }
}

pub enum ReturnType {
    None,
    Rax,
    Xmm0,
}

pub struct Trampoline(patchy_host::Trampoline);

impl Trampoline {
    pub fn new() -> Self {
        Self(patchy_host::Trampoline::new())
    }

    /// # Safety
    /// The site must provide aligned RSP and a valid Windows x64 call context.
    /// Both snippets must preserve RSP and fall through to the bridge.
    pub unsafe fn preserved_call(
        &mut self,
        callback: *const (),
        before_call: &[u8],
        after_call: &[u8],
        result: ReturnType,
    ) {
        self.0.call(CallBridge {
            callback,
            incoming_stack: StackAlignment::Aligned16,
            flags: FlagPolicy::KeepProduced,
            keep_return: match result {
                ReturnType::None => KeepReturn::Neither,
                ReturnType::Rax => KeepReturn::Rax,
                ReturnType::Xmm0 => KeepReturn::Xmm0,
            },
            before_call,
            after_call,
        });
    }
}

impl std::ops::Deref for Trampoline {
    type Target = patchy_host::Trampoline;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Trampoline {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
