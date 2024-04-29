#[macro_use]
extern crate enum_primitive;
extern crate byteorder;

use std::fmt;
use std::io;
use std::str::FromStr;
use windows::{ Win32::Foundation::*, Win32::System::SystemServices::*, Win32::System::Threading::GetCurrentProcessId, };
use windows::{ core::*, Win32::UI::WindowsAndMessaging::MessageBoxA, };

pub mod binary;

type Address = u16;
type Quantity = u16;
type Value = u16;

use crate::Error::{Exception, Io, InvalidData};

enum Function<'a> {
	ReadCoils(Address, Quantity),
	ReadDiscreteInputs(Address, Quantity),
	ReadHoldingRegisters(Address, Quantity),
	ReadInputRegisters(Address, Quantity),
	WriteSingleCoil(Address, Value),
	WriteSingleRegister(Address, Value),
	WriteMultipleCoils(Address, Quantity, &'a [u8]),
	WriteMultipleRegisters(Address, Quantity, &'a [u8]),
	WriteReadMultipleRegisters(Address, Quantity, &'a [u8], Address, Quantity),
}

impl<'a> Function<'a> {
	fn code(&self) -> u8 {
		match *self {
			Function::ReadCoils(_, _) => 0x01,
			Function::ReadDiscreteInputs(_, _) => 0x02,
			Function::ReadHoldingRegisters(_, _) => 0x03,
			Function::ReadInputRegisters(_, _) => 0x04,
			Function::WriteSingleCoil(_, _) => 0x05,
			Function::WriteSingleRegister(_, _) => 0x06,
			Function::WriteMultipleCoils(_, _, _) => 0x0f,
			Function::WriteMultipleRegisters(_, _, _) => 0x10,
			Function::WriteReadMultipleRegisters(_, _, _, _, _) => 0x17,
		}
		// ReadExceptionStatus     = 0x07,
		// ReportSlaveId           = 0x11,
		// MaskWriteRegister       = 0x16,
		// WriteAndReadRegisters   = 0x17
	}
}

enum_from_primitive! {
#[derive(Debug, PartialEq)]
/// Modbus exception codes returned from the server (slave).
pub enum ExceptionCode {
	IllegalFunction			= 0x01,
	IllegalDataAddress		= 0x02,
	IllegalDataValue		= 0x03,
	SlaveOrServerFailure	= 0x04,
	Acknowledge				= 0x05,
	SlaveOrServerBusy		= 0x06,
	NegativeAcknowledge		= 0x07,
	MemoryParity			= 0x08,
	NotDefined				= 0x09,
	GatewayPath				= 0x0a,
	GatewayTarget			= 0x0b
}
}

/// InvalidData reasons
#[derive(Debug)]
pub enum Reason {
	UnexpectedReplySize,
	BytecountNotEven,
	SendBufferEmpty,
	RecvBufferEmpty,
	SendBufferTooBig,
	DecodingError,
	EncodingError,
	InvalidByteorder,
	Custom(String),
}

/// Combination of modbus, I/O and data corruption errors.
#[derive(Debug)]
pub enum Error {
	Exception(ExceptionCode),
	Io(io::Error),
	InvalidResponse,
	InvalidData(Reason),
	InvalidFunction,
	ParseCoilError,
	ParseInfoError,
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		use crate::Error::*;

		match *self {
			Exception(ref code) => write!(f, "Modbus exception: {:?}", code),
			Io(ref err) => write!(f, "I/O error: {}", err),
			InvalidResponse => write!(f, "Invalid response"),
			InvalidData(ref reason) => write!(f, "Invalid data: {:?}", reason),
			InvalidFunction => write!(f, "Invalid modbus function"),
			ParseCoilError => write!(f, "Parse coil could not be parsed"),
			ParseInfoError => write!(f, "Failed parsing device info as utf8"),
		}
	}
}

impl std::error::Error for Error {
	fn description(&self) -> &str {
		use crate::Error::*;

		match *self {
			Exception(_) => "Modbus exception",
			Io(_) => "I/O error",
			InvalidResponse => "Invalid response",
			InvalidData(_) => "Invalid data",
			InvalidFunction => "Invalid modbus function",
			ParseCoilError => "Parse coil could not be parsed",
			ParseInfoError => "Failed parsing device info as utf8",
		}
	}

	fn cause(&self) -> Option<&dyn std::error::Error> {
		match *self {
			Error::Io(ref err) => Some(err),
			_ => None,
		}
	}
}

impl From<ExceptionCode> for Error {
	fn from(err: ExceptionCode) -> Error {
		Error::Exception(err)
	}
}

impl From<io::Error> for Error {
	fn from(err: io::Error) -> Error {
		Error::Io(err)
	}
}

/// Result type used to nofify success or failure in communication
pub type Result<T> = std::result::Result<T, Error>;

/// Single bit status values, used in read or write coil functions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Coil {
	On,
	Off,
}

impl Coil {
	fn code(self) -> u16 {
		match self {
			Coil::On => 0xff00,
			Coil::Off => 0x0000,
		}
	}
}

impl FromStr for Coil {
	type Err = Error;
	fn from_str(s: &str) -> Result<Coil> {
		if s == "On" {
			Ok(Coil::On)
		} else if s == "Off" {
			Ok(Coil::Off)
		} else {
			Err(Error::ParseCoilError)
		}
	}
}

impl From<bool> for Coil {
	fn from(b: bool) -> Coil {
		if b {
			Coil::On
		} else {
			Coil::Off
		}
	}
}

impl std::ops::Not for Coil {
	type Output = Coil;

	fn not(self) -> Coil {
		match self {
			Coil::On => Coil::Off,
			Coil::Off => Coil::On,
		}
	}
}

#[cfg(feature = "read-device-info")]
/// Types specific to the special ReadDeviceInfo function
pub mod mei {
	/**
	* Describes object standard conformity
	*
	* - **Basic** - Mandatory for Modbus standard conformity
	* - **Regular** - Defined in the standard, but implementation is optional
	* - **Extended** - Optional fields that are reserved for device specific information
	*/
	#[derive(Copy, Clone, Debug)]
	pub enum DeviceInfoCategory {
		Basic,
		Regular,
		Extended,
	}

	/**
	* Struct representing a device information object.
	*
	* The following object IDs are defined in the Modbus standard:
	* - **0x00** *BASIC* `VendorName`
	* - **0x01** *BASIC* `ProductCode`
	* - **0x02** *BASIC* `MajorMinorRevision`
	* - **0x03** *REGULAR* `VendorUrl`
	* - **0x04** *REGULAR* `ProductName`
	* - **0x05** *REGULAR* `ModelName`
	* - **0x06** *REGULAR* `UserApplicationName`
	* - **0x07 - 0x7F** *REGULAR* `Reserved`
	* - **0x80 - 0xFF** *EXTENDED* `Device Specific`
	*/
	#[derive(Clone, Debug)]
	pub struct DeviceInfoObject {
		id: u8,
		value: String,
	}
	impl DeviceInfoObject {
		pub fn new(obj_id: u8, value: String) -> Self {
			Self { id: obj_id, value }
		}
		pub fn to_string(&self) -> String {
			self.value.clone()
		}
		pub fn id(&self) -> u8 {
			self.id
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_coil_booleanness() {
		let a: Coil = true.into();
		assert_ne!(a, !a);
		assert_eq!(a, !!a);
		let b: Coil = false.into();
		assert_eq!(a, !b);
	}

	#[test]
	fn it_works() {
		let result = add(2, 2);
		assert_eq!(result, 4);
	}
}

#[no_mangle]
pub extern fn add(left: usize, right: usize) -> usize {
	left + right
}

#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn DllMain(
    dll_module: HINSTANCE,
    call_reason: u32,
    _: *mut ())
    -> bool
{
    match call_reason {
        DLL_PROCESS_ATTACH => attach(),
        DLL_PROCESS_DETACH => detach(),
        _ => ()
    }

    true
}

fn attach() {
    unsafe {
        let pid = GetCurrentProcessId();

        MessageBoxA(HWND(0),
            PCSTR(std::format!("Called from process: {}!\0", pid).as_ptr()),
            s!("modbus.dll"),
            Default::default()
        );
    };
}

fn detach() {
    unsafe {
        // Create a message box
        MessageBoxA(HWND(0),
            s!("Bye!"),
            s!("modbus.dll"),
            Default::default()
        );
    };
}

