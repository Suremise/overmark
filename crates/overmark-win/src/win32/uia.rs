//! A UI Automation client, for checks: what a screen reader (Narrator) finds in one of our own
//! windows, by name, and pressing one through its Invoke pattern, as Narrator does. It works on
//! the window it's given only; it's no global input (docs/TESTING.md).

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    IUIAutomationTogglePattern, TreeScope_Descendants, UIA_InvokePatternId, UIA_TogglePatternId,
};

/// An element a screen reader sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    /// Whether it can be pressed (UI Automation's Invoke pattern, or Toggle for an on/off
    /// button).
    pub invokable: bool,
    /// Whether it's enabled.
    pub enabled: bool,
    /// Where it is, in screen pixels.
    pub rect: super::Rect,
}

/// The client.
pub struct Uia {
    automation: IUIAutomation,
}

impl Uia {
    pub fn new() -> windows::core::Result<Uia> {
        // SAFETY: COM on this thread (already initialised is fine), then the system's UI
        // Automation object.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            Ok(Uia { automation })
        }
    }

    fn all(&self, hwnd: HWND) -> windows::core::Result<Vec<IUIAutomationElement>> {
        // SAFETY: calls on COM objects we hold, for a window handle (UI Automation checks it).
        unsafe {
            let root = self.automation.ElementFromHandle(hwnd)?;
            let all = root.FindAll(
                TreeScope_Descendants,
                &self.automation.CreateTrueCondition()?,
            )?;
            let mut out = Vec::new();
            for i in 0..all.Length()? {
                out.push(all.GetElement(i)?);
            }
            Ok(out)
        }
    }

    /// Every element under `hwnd`, in tree order.
    pub fn elements(&self, hwnd: HWND) -> windows::core::Result<Vec<Element>> {
        let mut out = Vec::new();
        for e in self.all(hwnd)? {
            // SAFETY: as above.
            unsafe {
                let name = e.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                let invokable = e
                    .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                    .is_ok()
                    || e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                        .is_ok();
                let enabled = e.CurrentIsEnabled().is_ok_and(|b| b.as_bool());
                let r = e.CurrentBoundingRectangle().unwrap_or_default();
                out.push(Element {
                    name,
                    invokable,
                    enabled,
                    rect: super::Rect::new(r.left, r.top, r.right, r.bottom),
                });
            }
        }
        Ok(out)
    }

    /// Presses the first element under `hwnd` named `name` (UI Automation's Invoke, or Toggle
    /// for an on/off button, as Narrator does). Whether there was one to press.
    pub fn invoke(&self, hwnd: HWND, name: &str) -> windows::core::Result<bool> {
        for e in self.all(hwnd)? {
            // SAFETY: as above.
            unsafe {
                if e.CurrentName().map(|b| b.to_string()).unwrap_or_default() != name {
                    continue;
                }
                if let Ok(p) =
                    e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                {
                    p.Invoke()?;
                    return Ok(true);
                }
                if let Ok(p) =
                    e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                {
                    p.Toggle()?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}
