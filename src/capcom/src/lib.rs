#![doc = include_str!("../../README.md")]
#![no_std]

use core::{arch::asm, ptr};

use wdk_sys::{
    DRIVER_OBJECT, FALSE, IO_NO_INCREMENT, IRP_MJ_CLOSE, IRP_MJ_CREATE, IRP_MJ_DEVICE_CONTROL,
    NT_SUCCESS, NTSTATUS, PAGED_CODE, PCUNICODE_STRING, PDEVICE_OBJECT, PDRIVER_OBJECT,
    PIO_STACK_LOCATION, PIRP, PUNICODE_STRING, PVOID, STATUS_SUCCESS, ULONG, UNICODE_STRING,
    ntddk::{
        IoCreateDevice, IoCreateSymbolicLink, IoDeleteDevice, IoDeleteSymbolicLink,
        IofCompleteRequest, KdRefreshDebuggerNotPresent, MmGetSystemRoutineAddress,
    },
};

const DEVICE_NAME: [u16; 18] = utf16_lit::utf16!("\\Device\\Htsysm72FB");
const LINK_NAME: [u16; 22] = utf16_lit::utf16!("\\DosDevices\\Htsysm72FB");

const DEVICE_TYPE: ULONG = 0xaa01;
const IOCTL_RUN_PAYLOAD: ULONG = (DEVICE_TYPE << 16) | 0x3044;

/// The entry point.
#[unsafe(link_section = "INIT")]
#[unsafe(export_name = "DriverEntry")]
extern "system" fn driver_entry(
    driver: &mut DRIVER_OBJECT,
    _registry_path: PCUNICODE_STRING,
) -> NTSTATUS {
    unsafe {
        // Break into a kernel debugger if present.
        if KdRefreshDebuggerNotPresent() == 0 {
            asm!("int3", options(nomem, nostack, preserves_flags));
        }

        let mut device_name = RTL_CONSTANT_STRING(&DEVICE_NAME);
        let mut device = ptr::null_mut();
        let status = IoCreateDevice(
            ptr::from_mut(driver),
            0,
            &raw mut device_name,
            DEVICE_TYPE,
            0,
            FALSE as _,
            &raw mut device,
        );
        assert!(NT_SUCCESS(status));

        let mut link_name = RTL_CONSTANT_STRING(&LINK_NAME);
        let status = IoCreateSymbolicLink(&raw mut link_name, &raw mut device_name);
        assert!(NT_SUCCESS(status));
    }

    driver.DriverUnload = Some(driver_unload);
    driver.MajorFunction[IRP_MJ_CREATE as usize] = Some(driver_open_close);
    driver.MajorFunction[IRP_MJ_CLOSE as usize] = Some(driver_open_close);
    driver.MajorFunction[IRP_MJ_DEVICE_CONTROL as usize] = Some(driver_ioctl);
    wdk::println!("Loaded the driver successfully");
    STATUS_SUCCESS
}

/// Handles the driver unload request.
#[unsafe(link_section = "PAGE")]
extern "C" fn driver_unload(driver: PDRIVER_OBJECT) {
    PAGED_CODE!();

    let mut link_name = RTL_CONSTANT_STRING(&LINK_NAME);
    unsafe {
        let _ = IoDeleteSymbolicLink(&raw mut link_name);
        IoDeleteDevice((*driver).DeviceObject);
    }
}

/// Handles the driver open and close request.
#[unsafe(link_section = "PAGE")]
extern "C" fn driver_open_close(_device: PDEVICE_OBJECT, irp: PIRP) -> NTSTATUS {
    PAGED_CODE!();
    unsafe {
        (*irp).IoStatus.__bindgen_anon_1.Status = STATUS_SUCCESS;
        (*irp).IoStatus.Information = 0;
        IofCompleteRequest(irp, IO_NO_INCREMENT as _);
    }
    STATUS_SUCCESS
}

/// Handles the driver IOCTL request.
#[unsafe(link_section = "PAGE")]
extern "C" fn driver_ioctl(_device: PDEVICE_OBJECT, irp: PIRP) -> NTSTATUS {
    PAGED_CODE!();
    unsafe {
        let stack = IoGetCurrentIrpStackLocation(irp);
        let control_code = (*stack).Parameters.DeviceIoControl.IoControlCode;

        // Execute payload if IOCTL_RUN_PAYLOAD is geven.
        if control_code == IOCTL_RUN_PAYLOAD {
            let buffer = (*irp).AssociatedIrp.SystemBuffer;
            let buffer = buffer.cast::<PayloadType>();
            run_payload(*buffer);
        }

        (*irp).IoStatus.__bindgen_anon_1.Status = STATUS_SUCCESS;
        IofCompleteRequest(irp, IO_NO_INCREMENT as _);
    }
    STATUS_SUCCESS
}

type PayloadType = unsafe extern "C" fn(unsafe extern "C" fn(PUNICODE_STRING) -> PVOID);

/// Executes `payload` without CR4.SMEP and interrupts.
unsafe fn run_payload(payload: PayloadType) {
    unsafe {
        let cr4 = disable_smep();
        payload(MmGetSystemRoutineAddress);
        restore_smep(cr4);
    }
}

/// Disables CR4.SMEP and disables interrupts.
unsafe fn disable_smep() -> u64 {
    const CR4_SMEP: u64 = 1 << 20;

    unsafe {
        asm!("cli", options(nomem, nostack));
        let cr4 = cr4();
        write_cr4(cr4 & !CR4_SMEP);
        cr4
    }
}

/// Restores CR4 and enables interrupts.
unsafe fn restore_smep(cr4: u64) {
    unsafe {
        write_cr4(cr4);
        asm!("sti", options(nomem, nostack));
    };
}

/// Reads from CR4.
unsafe fn cr4() -> u64 {
    let value;
    unsafe { asm!("mov {}, cr4", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// Writes to CR4.
unsafe fn write_cr4(value: u64) {
    unsafe { asm!("mov cr4, {}", in(reg) value, options(nomem, nostack, preserves_flags)) };
}

/// Returns a pointer to the current stack location in an I/O Request Packet (IRP).
#[expect(non_snake_case, clippy::inline_always)]
#[inline(always)]
unsafe fn IoGetCurrentIrpStackLocation(irp: PIRP) -> PIO_STACK_LOCATION {
    unsafe {
        assert!((*irp).CurrentLocation <= (*irp).StackCount + 1);
        (*irp)
            .Tail
            .Overlay
            .__bindgen_anon_2
            .__bindgen_anon_1
            .CurrentStackLocation
    }
}

/// Builds UNICODE_STRING with the UTF-16 string.
#[expect(non_snake_case, clippy::inline_always)]
#[inline(always)]
fn RTL_CONSTANT_STRING(utf16: &[u16]) -> UNICODE_STRING {
    let length_in_bytes = (utf16.len() * 2) as u16;
    UNICODE_STRING {
        Length: length_in_bytes,
        MaximumLength: length_in_bytes,
        Buffer: utf16.as_ptr().cast_mut(),
    }
}

/// Handles panic by breaking into a debugger if present and bug checking.
#[cfg(not(test))]
#[panic_handler]
fn handle_panic(info: &core::panic::PanicInfo<'_>) -> ! {
    const MANUALLY_INITIATED_CRASH: ULONG = 0x0000_00e2;

    wdk::println!("{info}");
    unsafe {
        if KdRefreshDebuggerNotPresent() == 0 {
            asm!("int3", options(nomem, nostack, preserves_flags));
        }
        wdk_sys::ntddk::KeBugCheck(MANUALLY_INITIATED_CRASH);
    }
}
