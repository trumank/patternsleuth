use windows::Win32::{
    Foundation::{
        DBG_PRINTEXCEPTION_C, DBG_PRINTEXCEPTION_WIDE_C, EXCEPTION_ACCESS_VIOLATION,
        EXCEPTION_ARRAY_BOUNDS_EXCEEDED, EXCEPTION_BREAKPOINT, EXCEPTION_ILLEGAL_INSTRUCTION,
        EXCEPTION_INT_DIVIDE_BY_ZERO, EXCEPTION_SINGLE_STEP, EXCEPTION_STACK_OVERFLOW, NTSTATUS,
    },
    System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS},
};

extern "system" fn vectored_exception_handler(exception_info: *mut EXCEPTION_POINTERS) -> i32 {
    unsafe {
        if !exception_info.is_null() {
            let exception_record = (*exception_info).ExceptionRecord;
            if !exception_record.is_null() {
                let exception_code = (*exception_record).ExceptionCode;
                let exception_address = (*exception_record).ExceptionAddress;

                let exception_name = match exception_code {
                    EXCEPTION_ACCESS_VIOLATION => "ACCESS_VIOLATION",
                    EXCEPTION_ILLEGAL_INSTRUCTION => "ILLEGAL_INSTRUCTION",
                    EXCEPTION_INT_DIVIDE_BY_ZERO => "INTEGER_DIVIDE_BY_ZERO",
                    EXCEPTION_ARRAY_BOUNDS_EXCEEDED => "ARRAY_BOUNDS_EXCEEDED",
                    EXCEPTION_STACK_OVERFLOW => "STACK_OVERFLOW",
                    EXCEPTION_BREAKPOINT => "BREAKPOINT",
                    EXCEPTION_SINGLE_STEP => "SINGLE_STEP",
                    DBG_PRINTEXCEPTION_WIDE_C | DBG_PRINTEXCEPTION_C | NTSTATUS(0x406D1388) => {
                        return 0;
                    }
                    _ => "UNKNOWN_EXCEPTION",
                };

                tracing::error!("=== UNHANDLED EXCEPTION ===");
                tracing::error!("Exception Code: 0x{:08X}", exception_code.0);
                tracing::error!("Exception Address: {:p}", exception_address);
                tracing::error!("Exception Type: {}", exception_name);
                crate::dump_backtrace();
            }
        }
    }

    0
}

pub unsafe fn setup_windows_exception_handler() {
    unsafe {
        let handle = AddVectoredExceptionHandler(1, Some(vectored_exception_handler));

        if handle.is_null() {
            tracing::warn!("Failed to install vectored exception handler");
        } else {
            tracing::info!("Vectored exception handler installed");
        }
    }
}
