//! Crate-specific error and result types, plus common conversions.

use ::std::fmt::{self, Display};

use ::windows::{
    core::{Error as Win32Error, HRESULT},
    Win32::Foundation::{SetLastError, NO_ERROR},
};

/// Result type returned by functions that call into Win32 API.
pub type Result<T> = ::std::result::Result<T, Error>;

/// Error type for functions that call into Win32 API. The error attempts to
/// pro-actively capture as much context as possible (error codes, system error
/// message strings, etc).
#[derive(Clone, Debug)]
pub struct Error {
    /// The underlying Win32 error. Implements [`Display`] to conveniently
    /// print any Win32 error codes or system error messages which were
    /// gathered at the point of the error.
    ///
    /// [`Display`]: std::fmt::Display
    underlying_error: Win32Error,

    /// The name of the Win32 API function which failed.
    function: Option<&'static str>,

    /// An optional context information which describes what was happening
    /// at the time error.
    context: Option<String>,
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            underlying_error,
            function,
            context,
        } = &self;

        if let Some(context) = context {
            write!(f, "{context}\nCaused by:\n    {underlying_error}")?;
        } else {
            write!(f, "{underlying_error}")?;
        }

        if let Some(function) = function {
            write!(f, " ({function})")?;
        }

        Ok(())
    }
}

impl Error {
    /// Returns the underlying Win32 error code, if any.
    pub fn code(&self) -> HRESULT {
        self.underlying_error.code()
    }
}

/// Gets the last Win32 error by calling [`GetLastError`].
///
/// Returns `Ok` if there is no last error (e.g. if the Win32 error code is
/// [`S_OK`]/[`ERROR_SUCCESS`]). Otherwise, returns `Err` with an inner error
/// that contains the system error message and code.
///
/// [`S_OK`]: https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-
/// [`ERROR_SUCCESS`]: https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-
/// [`GetLastError`]: https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-getlasterror
pub(crate) fn get_last_err() -> Result<()> {
    let last_err = Win32Error::from_win32();

    if last_err == Win32Error::OK {
        Ok(())
    } else {
        Err(Error {
            underlying_error: last_err,
            function: None,
            context: None,
        })
    }
}

/// Clears the last error by setting the system error value to [`S_OK`] /
/// [`ERROR_SUCCESS`].
///
/// [`S_OK`]: https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-
/// [`ERROR_SUCCESS`]: https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-
pub(crate) fn clear_last_error() {
    unsafe {
        SetLastError(NO_ERROR);
    }
}

/// A crate-private trait which allows context information to be attached to
/// fallible types.
///
/// This is useful to attach high level context information and track which
/// particular Win32 API function failed, something that might not be obvious
/// when relying on the inner windows error alone.
pub(crate) trait Context<T> {
    /// Attach the name of the function which failed to the error as additional
    /// context.
    fn function(self, function: &'static str) -> Result<T>
    where
        Self: Sized;

    /// Attach a context message to a fallible type and return crate error.
    fn context(self, ctx: impl AsRef<str>) -> Result<T>
    where
        Self: Sized;
}

impl<T> Context<T> for Result<T> {
    fn function(mut self, f: &'static str) -> Result<T>
    where
        Self: Sized,
    {
        if let Err(err) = &mut self {
            err.function = Some(f);
        }
        self
    }

    fn context(mut self, ctx: impl AsRef<str>) -> Result<T>
    where
        Self: Sized,
    {
        if let Err(err) = &mut self {
            err.context = Some(ctx.as_ref().to_owned());
        }
        self
    }
}

impl<T> Context<T> for Option<T> {
    fn function(self, function: &'static str) -> Result<T>
    where
        Self: Sized,
    {
        match self {
            Some(v) => Ok(v),
            None => Err(Error {
                underlying_error: Win32Error::from_win32(),
                function: Some(function),
                context: None,
            }),
        }
    }

    fn context(self, ctx: impl AsRef<str>) -> Result<T>
    where
        Self: Sized,
    {
        match self {
            Some(v) => Ok(v),
            None => Err(Error {
                underlying_error: Win32Error::from_win32(),
                context: Some(ctx.as_ref().to_owned()),
                function: None,
            }),
        }
    }
}

impl<T> Context<T> for ::std::result::Result<T, Win32Error> {
    fn function(self, function: &'static str) -> Result<T> {
        self.map_err(|source| Error {
            underlying_error: source,
            context: None,
            function: Some(function),
        })
    }

    fn context(self, ctx: impl AsRef<str>) -> Result<T> {
        self.map_err(|source| Error {
            underlying_error: source,
            context: Some(ctx.as_ref().to_owned()),
            function: None,
        })
    }
}
