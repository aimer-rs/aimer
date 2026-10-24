#![deny(missing_docs)]

//! Deterministic, UI-thread-local form state and validation primitives.
//!
//! This crate deliberately keeps input presentation hints separate from
//! validation. A field may ask a platform for a numeric keyboard while still
//! requiring an explicit validator to decide whether its value is valid.

use std::fmt;

/// A stable identifier for a field in a [`Form`].
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FieldId(String);

impl FieldId {
    /// Creates an identifier from an application-defined name.
    ///
    /// Empty identifiers are accepted here so a field can be assembled with
    /// the same builder shape as every other field. [`Form::add_field`] rejects
    /// them before they enter a form.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the identifier text.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether the identifier has no text.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl AsRef<str> for FieldId {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for FieldId {
    #[inline]
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl From<String> for FieldId {
    #[inline]
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl fmt::Display for FieldId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An input presentation hint. It never decides whether a value is valid.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InputHint {
    /// Ordinary text input.
    #[default]
    Text,
    /// Password or otherwise obscured input.
    Password,
    /// Email keyboard and autocomplete hint.
    Email,
    /// Telephone keyboard hint.
    Tel,
    /// URL keyboard and autocomplete hint.
    Url,
    /// Search keyboard hint.
    Search,
    /// Numeric keyboard hint; it does not parse or validate the value.
    Number,
    /// Calendar date hint.
    Date,
    /// Clock time hint.
    Time,
    /// Local date and time hint.
    DateTimeLocal,
    /// Month-only date hint.
    Month,
    /// Week-only date hint.
    Week,
    /// Non-editable hidden-control hint.
    Hidden,
    /// Reset-control hint.
    Reset,
    /// Submit-control hint.
    Submit,
    /// Image-control hint.
    Image,
    /// File-control hint.
    File,
}

impl InputHint {
    /// Returns the conventional HTML input type for adapters that support it.
    ///
    /// Adapters may intentionally fall back to `text` when a platform does
    /// not implement a particular control. This mapping is only a hint and is
    /// never consulted by [`FormField::validate`].
    pub const fn html_type(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Password => "password",
            Self::Email => "email",
            Self::Tel => "tel",
            Self::Url => "url",
            Self::Search => "search",
            Self::Number => "number",
            Self::Date => "date",
            Self::Time => "time",
            Self::DateTimeLocal => "datetime-local",
            Self::Month => "month",
            Self::Week => "week",
            Self::Hidden => "hidden",
            Self::Reset => "reset",
            Self::Submit => "submit",
            Self::Image => "image",
            Self::File => "file",
        }
    }

    /// Returns whether this hint normally describes editable text.
    pub const fn is_editable(self) -> bool {
        matches!(
            self,
            Self::Text
                | Self::Password
                | Self::Email
                | Self::Tel
                | Self::Url
                | Self::Search
                | Self::Number
                | Self::Date
                | Self::Time
                | Self::DateTimeLocal
                | Self::Month
                | Self::Week
        )
    }
}

/// The result of running a field's synchronous validators.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationState {
    /// The value has changed or has not been checked yet.
    Unvalidated,
    /// All synchronous validators accepted the value.
    Valid,
    /// One or more synchronous validators rejected the value.
    Invalid,
    /// An externally executed asynchronous check is in flight.
    Pending,
}

/// A validation message returned by a synchronous or asynchronous validator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError {
    message: String,
}

impl ValidationError {
    /// Creates a validation error with a user-facing message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the user-facing validation message.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for ValidationError {}

/// A synchronous validator supplied by an application.
pub trait Validator {
    /// Checks `value`, returning one error when it is not acceptable.
    fn validate(&self, value: &str) -> Result<(), ValidationError>;
}

impl<F> Validator for F
where
    F: Fn(&str) -> Result<(), ValidationError>,
{
    #[inline]
    fn validate(&self, value: &str) -> Result<(), ValidationError> {
        self(value)
    }
}

/// A result produced by an external asynchronous validator.
pub type AsyncValidationResult = Result<(), Vec<ValidationError>>;

/// A form-level error caused by field registration or lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormError {
    /// A field with no identifier cannot be registered.
    EmptyFieldId,
    /// A field identifier was registered more than once.
    DuplicateField(FieldId),
    /// An operation named a field that is not in the form.
    UnknownField(FieldId),
}

impl fmt::Display for FormError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFieldId => f.write_str("a form field identifier cannot be empty"),
            Self::DuplicateField(id) => write!(f, "form field {id} is already registered"),
            Self::UnknownField(id) => write!(f, "form field {id} is not registered"),
        }
    }
}

impl std::error::Error for FormError {}

/// The result of a form submission attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmitResult {
    /// Every field passed its synchronous validators.
    Accepted,
    /// Submission was rejected; the returned identifier is the first field
    /// with an error in registration order.
    Rejected {
        /// The first invalid field's configured focus target.
        first_error: FieldId,
    },
}

impl SubmitResult {
    /// Returns whether the submission was accepted.
    #[inline]
    pub const fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    /// Returns the first invalid field, if submission was rejected.
    #[inline]
    pub fn first_error(&self) -> Option<&FieldId> {
        match self {
            Self::Accepted => None,
            Self::Rejected { first_error } => Some(first_error),
        }
    }
}

/// A request handed to an external asynchronous validator.
///
/// The request contains an immutable value and a generation. An application
/// can run the check on any executor and later pass this request back to
/// [`Form::resolve_async_validation`]. If the user edited or reset the field
/// meanwhile, the generation no longer matches and the result is ignored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsyncValidationRequest {
    field: FieldId,
    value: String,
    generation: u64,
}

impl AsyncValidationRequest {
    /// Returns the field whose value was checked.
    #[inline]
    pub fn field(&self) -> &FieldId {
        &self.field
    }

    /// Returns the immutable value captured for the check.
    #[inline]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the opaque generation used to reject stale results.
    #[inline]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// Describes whether an asynchronous validation result changed form state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncValidationResolution {
    /// The result matched the current field value and was applied.
    Applied,
    /// The result was for an older value and was deliberately ignored.
    IgnoredStale,
}

/// A form field with a value, validation rules, and interaction state.
pub struct FormField {
    id: FieldId,
    focus_target: FieldId,
    value: String,
    initial_value: String,
    input_hint: InputHint,
    validators: Vec<Box<dyn Validator>>,
    touched: bool,
    validation_state: ValidationState,
    errors: Vec<ValidationError>,
    generation: u64,
}

impl fmt::Debug for FormField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FormField")
            .field("id", &self.id)
            .field("focus_target", &self.focus_target)
            .field("value", &self.value)
            .field("initial_value", &self.initial_value)
            .field("input_hint", &self.input_hint)
            .field("validator_count", &self.validators.len())
            .field("touched", &self.touched)
            .field("dirty", &self.dirty())
            .field("validation_state", &self.validation_state)
            .field("errors", &self.errors)
            .finish()
    }
}

impl FormField {
    /// Creates a field with an initially unvalidated value.
    pub fn new(id: impl Into<FieldId>, initial_value: impl Into<String>) -> Self {
        let id = id.into();
        let initial_value = initial_value.into();
        Self {
            focus_target: id.clone(),
            id,
            value: initial_value.clone(),
            initial_value,
            input_hint: InputHint::default(),
            validators: Vec::new(),
            touched: false,
            validation_state: ValidationState::Unvalidated,
            errors: Vec::new(),
            generation: 0,
        }
    }

    /// Replaces the target an application should focus when this field is the
    /// first invalid field. The target is opaque to the form crate.
    #[inline]
    pub fn focus_target(mut self, target: impl Into<FieldId>) -> Self {
        self.focus_target = target.into();
        self
    }

    /// Sets a presentation hint without installing validation behavior.
    #[inline]
    pub fn input_hint(mut self, hint: InputHint) -> Self {
        self.input_hint = hint;
        self
    }

    /// Adds a synchronous validator in declaration order.
    #[inline]
    pub fn validator(mut self, validator: impl Validator + 'static) -> Self {
        self.add_validator(validator);
        self
    }

    /// Adds a synchronous validator after the field has been constructed.
    #[inline]
    pub fn add_validator(&mut self, validator: impl Validator + 'static) {
        self.validators.push(Box::new(validator));
        self.invalidate_validation();
    }

    /// Returns this field's stable identifier.
    #[inline]
    pub fn id(&self) -> &FieldId {
        &self.id
    }

    /// Returns the current value.
    #[inline]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the value restored by [`Self::reset`].
    #[inline]
    pub fn initial_value(&self) -> &str {
        &self.initial_value
    }

    /// Returns the presentation hint configured for this field.
    #[inline]
    pub const fn input_hint_value(&self) -> InputHint {
        self.input_hint
    }

    /// Returns the opaque focus target for this field.
    #[inline]
    pub fn focus_target_value(&self) -> &FieldId {
        &self.focus_target
    }

    /// Returns whether the field has been touched by the user or by submit.
    #[inline]
    pub const fn touched(&self) -> bool {
        self.touched
    }

    /// Returns whether the current value differs from the initial value.
    #[inline]
    pub fn dirty(&self) -> bool {
        self.value != self.initial_value
    }

    /// Returns the latest validation state.
    #[inline]
    pub fn validation_state(&self) -> &ValidationState {
        &self.validation_state
    }

    /// Returns all validation errors in validator declaration order.
    #[inline]
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Returns whether the field has a current successful validation result.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.validation_state == ValidationState::Valid
    }

    /// Replaces the value as a programmatic or controlled update.
    ///
    /// A changed value becomes dirty and unvalidated. This method does not mark
    /// the field touched; use [`Self::set_user_value`] or [`Self::mark_touched`]
    /// when the update came from an interaction.
    #[inline]
    pub fn set_value(&mut self, value: impl Into<String>) -> bool {
        let value = value.into();
        if self.value == value {
            return false;
        }
        self.value = value;
        self.invalidate_validation();
        true
    }

    /// Applies a user edit, marking the field touched and invalidating its
    /// previous validation result when the value changed.
    #[inline]
    pub fn set_user_value(&mut self, value: impl Into<String>) -> bool {
        let was_touched = self.touched;
        self.touched = true;
        let changed = self.set_value(value);
        changed || !was_touched
    }

    /// Marks the field touched without changing its value.
    #[inline]
    pub fn mark_touched(&mut self) {
        self.touched = true;
    }

    /// Marks the field touched on a focus-loss boundary.
    #[inline]
    pub fn blur(&mut self) {
        self.mark_touched();
    }

    /// Runs every synchronous validator and aggregates all errors.
    pub fn validate(&mut self) -> ValidationState {
        self.next_generation();
        let mut errors = Vec::new();
        for validator in &self.validators {
            if let Err(error) = validator.validate(&self.value) {
                errors.push(error);
            }
        }
        self.errors = errors;
        self.validation_state = if self.errors.is_empty() {
            ValidationState::Valid
        } else {
            ValidationState::Invalid
        };
        self.validation_state.clone()
    }

    /// Starts an externally executed asynchronous validation check.
    pub fn begin_async_validation(&mut self) -> AsyncValidationRequest {
        let generation = self.next_generation();
        self.errors.clear();
        self.validation_state = ValidationState::Pending;
        AsyncValidationRequest {
            field: self.id.clone(),
            value: self.value.clone(),
            generation,
        }
    }

    /// Applies an asynchronous result if it still belongs to this value.
    ///
    /// A result for another field, generation, or value is ignored. Applying a
    /// matching result advances the generation so a duplicate callback cannot
    /// mutate state twice.
    pub fn apply_async_validation(
        &mut self,
        request: &AsyncValidationRequest,
        result: AsyncValidationResult,
    ) -> AsyncValidationResolution {
        if request.field != self.id
            || request.generation != self.generation
            || request.value != self.value
        {
            return AsyncValidationResolution::IgnoredStale;
        }

        self.next_generation();
        self.errors = result.err().unwrap_or_default();
        self.validation_state = if self.errors.is_empty() {
            ValidationState::Valid
        } else {
            ValidationState::Invalid
        };
        AsyncValidationResolution::Applied
    }

    /// Restores the initial value and clears touched, dirty, pending, and error
    /// state.
    pub fn reset(&mut self) {
        self.value.clone_from(&self.initial_value);
        self.touched = false;
        self.errors.clear();
        self.validation_state = ValidationState::Unvalidated;
        self.next_generation();
    }

    fn invalidate_validation(&mut self) {
        self.next_generation();
        self.errors.clear();
        self.validation_state = ValidationState::Unvalidated;
    }

    fn next_generation(&mut self) -> u64 {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("exhausted form validation generations");
        self.generation
    }
}

/// A deterministic collection of [`FormField`] values.
///
/// Fields retain registration order. That order is used for submission and for
/// selecting the first error, so focus behavior does not depend on hash-map
/// iteration or platform timing.
#[derive(Debug, Default)]
pub struct Form {
    fields: Vec<FormField>,
    submitted: bool,
}

impl Form {
    /// Creates an empty form.
    #[inline]
    pub const fn new() -> Self {
        Self {
            fields: Vec::new(),
            submitted: false,
        }
    }

    /// Registers a field, preserving its position as the form's validation
    /// order.
    pub fn add_field(&mut self, field: FormField) -> Result<(), FormError> {
        if field.id.is_empty() {
            return Err(FormError::EmptyFieldId);
        }
        if self.fields.iter().any(|current| current.id == field.id) {
            return Err(FormError::DuplicateField(field.id.clone()));
        }
        self.fields.push(field);
        Ok(())
    }

    /// Returns all fields in deterministic registration order.
    #[inline]
    pub fn fields(&self) -> &[FormField] {
        &self.fields
    }

    /// Looks up a field by identifier.
    #[inline]
    pub fn field(&self, id: impl AsRef<str>) -> Option<&FormField> {
        let id = id.as_ref();
        self.fields.iter().find(|field| field.id.as_str() == id)
    }

    /// Looks up a mutable field by identifier.
    pub fn field_mut(&mut self, id: impl AsRef<str>) -> Result<&mut FormField, FormError> {
        let id = id.as_ref();
        self.fields
            .iter_mut()
            .find(|field| field.id.as_str() == id)
            .ok_or_else(|| FormError::UnknownField(FieldId::new(id)))
    }

    /// Replaces a field value without marking it touched.
    pub fn set_value(
        &mut self,
        id: impl AsRef<str>,
        value: impl Into<String>,
    ) -> Result<bool, FormError> {
        self.field_mut(id).map(|field| field.set_value(value))
    }

    /// Applies a user edit to a field and marks it touched.
    pub fn set_user_value(
        &mut self,
        id: impl AsRef<str>,
        value: impl Into<String>,
    ) -> Result<bool, FormError> {
        self.field_mut(id).map(|field| field.set_user_value(value))
    }

    /// Marks a field touched, usually from a blur or submit event.
    pub fn mark_touched(&mut self, id: impl AsRef<str>) -> Result<(), FormError> {
        self.field_mut(id).map(FormField::mark_touched)
    }

    /// Validates one field and returns its new state.
    pub fn validate_field(&mut self, id: impl AsRef<str>) -> Result<ValidationState, FormError> {
        self.field_mut(id).map(FormField::validate)
    }

    /// Validates every field in registration order.
    pub fn validate(&mut self) -> bool {
        for field in &mut self.fields {
            field.validate();
        }
        self.is_valid()
    }

    /// Marks every field touched, validates synchronously, and returns the
    /// first focus target when any field is invalid.
    pub fn submit(&mut self) -> SubmitResult {
        self.submitted = true;
        for field in &mut self.fields {
            field.mark_touched();
            field.validate();
        }
        match self.first_error_focus_target() {
            Some(first_error) => SubmitResult::Rejected {
                first_error: first_error.clone(),
            },
            None => SubmitResult::Accepted,
        }
    }

    /// Returns whether submit has been attempted since the last reset.
    #[inline]
    pub const fn submitted(&self) -> bool {
        self.submitted
    }

    /// Returns whether every registered field has a current valid result.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.fields.iter().all(FormField::is_valid)
    }

    /// Returns whether any field currently has one or more errors.
    #[inline]
    pub fn has_errors(&self) -> bool {
        self.fields.iter().any(|field| !field.errors.is_empty())
    }

    /// Returns the first invalid field's opaque focus target in registration
    /// order.
    #[inline]
    pub fn first_error_focus_target(&self) -> Option<&FieldId> {
        self.fields
            .iter()
            .find(|field| field.validation_state == ValidationState::Invalid)
            .map(|field| &field.focus_target)
    }

    /// Returns a stable snapshot of all values in registration order.
    pub fn values(&self) -> Vec<(FieldId, String)> {
        self.fields
            .iter()
            .map(|field| (field.id.clone(), field.value.clone()))
            .collect()
    }

    /// Begins asynchronous validation for a field.
    pub fn begin_async_validation(
        &mut self,
        id: impl AsRef<str>,
    ) -> Result<AsyncValidationRequest, FormError> {
        self.field_mut(id).map(FormField::begin_async_validation)
    }

    /// Applies an asynchronous result, ignoring stale results safely.
    pub fn resolve_async_validation(
        &mut self,
        request: &AsyncValidationRequest,
        result: AsyncValidationResult,
    ) -> Result<AsyncValidationResolution, FormError> {
        self.field_mut(request.field.as_str())
            .map(|field| field.apply_async_validation(request, result))
    }

    /// Restores every field to its initial value and clears submission state.
    pub fn reset(&mut self) {
        for field in &mut self.fields {
            field.reset();
        }
        self.submitted = false;
    }
}

/// Creates a validator that rejects blank or whitespace-only values.
pub fn required() -> impl Validator {
    |value: &str| {
        if value.trim().is_empty() {
            Err(ValidationError::new("This field is required"))
        } else {
            Ok(())
        }
    }
}

/// Creates a validator that requires at least `minimum` Unicode scalar values.
pub fn min_length(minimum: usize) -> impl Validator {
    move |value: &str| {
        if value.chars().count() < minimum {
            Err(ValidationError::new(format!(
                "Use at least {minimum} characters"
            )))
        } else {
            Ok(())
        }
    }
}

/// Creates a validator that accepts a finite number or an empty optional value.
///
/// Use [`required`] as a separate rule when an empty number is not allowed.
pub fn number() -> impl Validator {
    |value: &str| {
        if value.trim().is_empty() {
            return Ok(());
        }
        match value.trim().parse::<f64>() {
            Ok(number) if number.is_finite() => Ok(()),
            _ => Err(ValidationError::new("Enter a valid number")),
        }
    }
}

/// Creates a small, platform-neutral email-shape validator.
///
/// This intentionally checks only the local/domain shape. Applications that
/// need deliverability or server-side uniqueness should add an asynchronous
/// validator through [`Form::begin_async_validation`].
pub fn email() -> impl Validator {
    |value: &str| {
        let mut parts = value.split('@');
        let local = parts.next().unwrap_or_default();
        let domain = parts.next().unwrap_or_default();
        let has_one_at = parts.next().is_none();
        if !local.is_empty()
            && !domain.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && has_one_at
        {
            Ok(())
        } else {
            Err(ValidationError::new("Enter a valid email address"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validators_aggregate_errors_and_leave_input_hints_non_validating() {
        let mut field = FormField::new("age", "not-a-number")
            .input_hint(InputHint::Number)
            .validator(number())
            .validator(min_length(20));

        assert_eq!(field.input_hint_value(), InputHint::Number);
        assert_eq!(field.validation_state(), &ValidationState::Unvalidated);
        assert_eq!(field.validate(), ValidationState::Invalid);
        assert_eq!(
            field.errors(),
            &[
                ValidationError::new("Enter a valid number"),
                ValidationError::new("Use at least 20 characters"),
            ]
        );
    }

    #[test]
    fn field_edit_touch_validate_and_reset_transitions_are_deterministic() {
        let mut field = FormField::new("name", "Ada").validator(required());

        assert!(!field.touched());
        assert!(!field.dirty());
        assert_eq!(field.validate(), ValidationState::Valid);

        assert!(field.set_user_value(""));
        assert!(field.touched());
        assert!(field.dirty());
        assert_eq!(field.validation_state(), &ValidationState::Unvalidated);
        assert_eq!(field.validate(), ValidationState::Invalid);

        field.reset();
        assert_eq!(field.value(), "Ada");
        assert!(!field.touched());
        assert!(!field.dirty());
        assert!(field.errors().is_empty());
        assert_eq!(field.validation_state(), &ValidationState::Unvalidated);
    }

    #[test]
    fn submit_marks_all_fields_and_returns_the_first_configured_focus_target() {
        let mut form = Form::new();
        form.add_field(
            FormField::new("email", "")
                .focus_target("email-focus")
                .validator(required())
                .validator(email()),
        )
        .unwrap();
        form.add_field(FormField::new("name", "").validator(required()))
            .unwrap();

        assert_eq!(
            form.submit(),
            SubmitResult::Rejected {
                first_error: FieldId::from("email-focus"),
            }
        );
        assert!(form.submitted());
        assert!(form.field("email").unwrap().touched());
        assert!(form.field("name").unwrap().touched());
        assert_eq!(
            form.first_error_focus_target().unwrap().as_str(),
            "email-focus"
        );
    }

    #[test]
    fn stale_async_results_cannot_replace_a_newer_user_value() {
        let mut form = Form::new();
        form.add_field(FormField::new("username", "old")).unwrap();

        let old_request = form.begin_async_validation("username").unwrap();
        assert_eq!(
            form.field("username").unwrap().validation_state(),
            &ValidationState::Pending
        );
        form.set_user_value("username", "new").unwrap();

        assert_eq!(
            form.resolve_async_validation(
                &old_request,
                Err(vec![ValidationError::new("old value is taken")]),
            )
            .unwrap(),
            AsyncValidationResolution::IgnoredStale
        );
        assert_eq!(form.field("username").unwrap().value(), "new");
        assert!(form.field("username").unwrap().errors().is_empty());

        let new_request = form.begin_async_validation("username").unwrap();
        assert_eq!(
            form.resolve_async_validation(&new_request, Ok(())).unwrap(),
            AsyncValidationResolution::Applied
        );
        assert!(form.field("username").unwrap().is_valid());
    }

    #[test]
    fn registration_and_lookup_errors_are_explicit() {
        let mut form = Form::new();
        assert_eq!(
            form.add_field(FormField::new("", "")),
            Err(FormError::EmptyFieldId)
        );
        form.add_field(FormField::new("name", "Ada")).unwrap();
        assert_eq!(
            form.add_field(FormField::new("name", "Grace")),
            Err(FormError::DuplicateField(FieldId::from("name")))
        );
        assert_eq!(
            form.set_value("missing", "value"),
            Err(FormError::UnknownField(FieldId::from("missing")))
        );
    }

    #[test]
    fn utility_validators_cover_boundaries_without_inventing_input_validity() {
        let mut field = FormField::new("amount", "")
            .validator(required())
            .validator(number());
        assert_eq!(field.validate(), ValidationState::Invalid);

        field.set_value("0");
        assert_eq!(field.validate(), ValidationState::Valid);

        field.set_value("1e309");
        assert_eq!(field.validate(), ValidationState::Invalid);

        assert_eq!(InputHint::Number.html_type(), "number");
        assert!(!InputHint::Submit.is_editable());
    }
}
