//! Protobuf text generator - converts FileDescriptorProto to canonical proto text.
//!
//! This module ports Google's C++ DebugStringWithOptions implementation to Rust,
//! ensuring stable, deterministic output for proto descriptors.

use anyhow::Result;
use protobuf::descriptor::{
    field_descriptor_proto::{Label, Type},
    DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
    FileDescriptorProto, MethodDescriptorProto, OneofDescriptorProto, ServiceDescriptorProto,
};
use std::fmt::Write;

/// Version of the text generation algorithm.
/// Increment when output format changes to ensure reproducibility.
pub const TEXT_GENERATOR_VERSION: &str = "1.0.0";

/// Information about a map field
struct MapFieldInfo {
    key_type: String,
    value_type: String,
}

/// Configuration for text generation.
#[derive(Debug, Clone)]
pub struct TextGeneratorOptions {
    /// Indent size in spaces (default: 2)
    pub indent_size: usize,
    /// Sort messages by name (default: true for determinism)
    pub sort_messages: bool,
    /// Sort enums by name (default: true for determinism)
    pub sort_enums: bool,
    /// Sort services by name (default: true for determinism)
    pub sort_services: bool,
}

impl Default for TextGeneratorOptions {
    fn default() -> Self {
        Self {
            indent_size: 2,
            sort_messages: true,
            sort_enums: true,
            sort_services: true,
        }
    }
}

/// Generates canonical protobuf text from descriptors.
pub struct TextGenerator {
    options: TextGeneratorOptions,
    output: String,
    indent_level: usize,
    current_message: Option<DescriptorProto>,
    current_file: Option<FileDescriptorProto>,
}

impl TextGenerator {
    pub fn new(options: TextGeneratorOptions) -> Self {
        Self {
            options,
            output: String::new(),
            indent_level: 0,
            current_message: None,
            current_file: None,
        }
    }

    pub fn with_default() -> Self {
        Self::new(TextGeneratorOptions::default())
    }

    /// Main entry point: format a FileDescriptorProto to canonical proto text.
    pub fn format_file(&mut self, file: &FileDescriptorProto) -> Result<String> {
        self.output.clear();
        self.indent_level = 0;
        self.current_file = Some(file.clone());
        // 1. Syntax (default to proto2 if not specified)
        let syntax = file.syntax.as_deref().unwrap_or("proto2");
        if !syntax.is_empty() {
            writeln!(self.output, "syntax = \"{syntax}\";")?;
            self.write_newline();
        }

        // 2. Package
        if let Some(package) = file.package.as_ref() {
            if !package.is_empty() {
                writeln!(self.output, "package {package};")?;
                self.write_newline();
            }
        }

        // 3. Imports (sorted for determinism)
        self.write_imports(file)?;

        // 4. File-level options
        self.write_file_options(file)?;

        // 5. Messages (sorted by name if enabled)
        self.write_messages(file, syntax)?;

        // 6. Enums (sorted by name if enabled)
        self.write_enums(file)?;

        // 7. Services (sorted by name if enabled)
        self.write_services(file)?;

        // 8. Extensions (proto2)
        self.write_extensions(file, syntax)?;

        Ok(self.output.clone())
    }

    // ========== Helper Methods ==========

    fn escape_string(s: &str) -> String {
        let mut result = String::new();
        for ch in s.chars() {
            match ch {
                '\\' => result.push_str("\\\\"),
                '"' => result.push_str("\\\""),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                _ => result.push(ch),
            }
        }
        result
    }

    fn escape_bytes(b: &[u8]) -> String {
        let mut out = String::new();
        for &byte in b {
            match byte {
                b'"' => out.push_str("\\\""),
                b'\\' => out.push_str("\\\\"),
                b'\n' => out.push_str("\\n"),
                b'\r' => out.push_str("\\r"),
                b'\t' => out.push_str("\\t"),
                0x20..=0x7E => out.push(byte as char), // printable ASCII
                _ => {
                    // Use octal escapes (\NNN) as commonly used in DebugString outputs
                    use std::fmt::Write as _;
                    write!(&mut out, "\\{byte:03o}").unwrap();
                }
            }
        }
        out
    }

    fn write_indent(&mut self) {
        let spaces = " ".repeat(self.indent_level * self.options.indent_size);
        self.output.push_str(&spaces);
    }

    fn write_newline(&mut self) {
        self.output.push('\n');
    }

    fn indent(&mut self) {
        self.indent_level += 1;
    }

    fn dedent(&mut self) {
        self.indent_level = self.indent_level.saturating_sub(1);
    }

    // ========== Imports ==========

    fn write_imports(&mut self, file: &FileDescriptorProto) -> Result<()> {
        if file.dependency.is_empty()
            && file.public_dependency.is_empty()
            && file.weak_dependency.is_empty()
        {
            return Ok(());
        }

        let mut imports = Vec::new();

        // Regular imports
        for dep in file.dependency.iter() {
            imports.push((dep.as_str(), false, false)); // (path, is_public, is_weak)
        }

        // Mark public imports
        for &idx in file.public_dependency.iter() {
            if let Some(item) = imports.get_mut(idx as usize) {
                item.1 = true;
            }
        }

        // Mark weak imports
        for &idx in file.weak_dependency.iter() {
            if let Some(item) = imports.get_mut(idx as usize) {
                item.2 = true;
            }
        }

        // Sort: by kind (normal=0, public=1, weak=2), then by path
        imports.sort_by(|a, b| {
            let rank = |is_public: bool, is_weak: bool| {
                if is_public {
                    1
                } else if is_weak {
                    2
                } else {
                    0
                }
            };
            let ar = rank(a.1, a.2);
            let br = rank(b.1, b.2);
            match ar.cmp(&br) {
                std::cmp::Ordering::Equal => a.0.cmp(b.0),
                other => other,
            }
        });

        // Write imports
        for (path, is_public, is_weak) in imports {
            if is_public {
                writeln!(self.output, "import public \"{path}\";")?;
            } else if is_weak {
                writeln!(self.output, "import weak \"{path}\";")?;
            } else {
                writeln!(self.output, "import \"{path}\";")?;
            }
        }

        self.write_newline();
        Ok(())
    }

    // ========== File Options ==========

    fn write_file_options(&mut self, file: &FileDescriptorProto) -> Result<()> {
        if let Some(options) = file.options.as_ref() {
            // Write common file options in sorted order
            let mut opts = Vec::new();

            if let Some(val) = options.java_package.as_ref() {
                opts.push(format!(
                    "option java_package = \"{}\";",
                    Self::escape_string(val)
                ));
            }
            if let Some(val) = options.java_outer_classname.as_ref() {
                opts.push(format!(
                    "option java_outer_classname = \"{}\";",
                    Self::escape_string(val)
                ));
            }
            if let Some(val) = options.java_multiple_files {
                opts.push(format!("option java_multiple_files = {val};"));
            }
            if let Some(val) = options.java_string_check_utf8 {
                opts.push(format!("option java_string_check_utf8 = {val};"));
            }
            if let Some(val) = options.go_package.as_ref() {
                opts.push(format!(
                    "option go_package = \"{}\";",
                    Self::escape_string(val)
                ));
            }
            if let Some(val) = options.optimize_for {
                // Enum OptimizeMode printing: SPEED, CODE_SIZE, LITE_RUNTIME
                let mode = match val.value() {
                    x if x == protobuf::descriptor::file_options::OptimizeMode::SPEED as i32 => {
                        "SPEED"
                    }
                    x if x
                        == protobuf::descriptor::file_options::OptimizeMode::CODE_SIZE as i32 =>
                    {
                        "CODE_SIZE"
                    }
                    x if x
                        == protobuf::descriptor::file_options::OptimizeMode::LITE_RUNTIME
                            as i32 =>
                    {
                        "LITE_RUNTIME"
                    }
                    _ => "SPEED",
                };
                opts.push(format!("option optimize_for = {mode};"));
            }
            if let Some(val) = options.cc_enable_arenas {
                opts.push(format!("option cc_enable_arenas = {val};"));
            }
            if let Some(val) = options.cc_generic_services {
                opts.push(format!("option cc_generic_services = {val};"));
            }
            if let Some(val) = options.java_generic_services {
                opts.push(format!("option java_generic_services = {val};"));
            }
            if let Some(val) = options.py_generic_services {
                opts.push(format!("option py_generic_services = {val};"));
            }
            if let Some(val) = options.objc_class_prefix.as_ref() {
                opts.push(format!(
                    "option objc_class_prefix = \"{}\";",
                    Self::escape_string(val)
                ));
            }
            if let Some(val) = options.csharp_namespace.as_ref() {
                opts.push(format!(
                    "option csharp_namespace = \"{}\";",
                    Self::escape_string(val)
                ));
            }
            if let Some(val) = options.swift_prefix.as_ref() {
                opts.push(format!(
                    "option swift_prefix = \"{}\";",
                    Self::escape_string(val)
                ));
            }
            if let Some(val) = options.php_class_prefix.as_ref() {
                opts.push(format!(
                    "option php_class_prefix = \"{}\";",
                    Self::escape_string(val)
                ));
            }
            if let Some(val) = options.php_namespace.as_ref() {
                opts.push(format!(
                    "option php_namespace = \"{}\";",
                    Self::escape_string(val)
                ));
            }
            if let Some(val) = options.php_metadata_namespace.as_ref() {
                opts.push(format!(
                    "option php_metadata_namespace = \"{}\";",
                    Self::escape_string(val)
                ));
            }
            if let Some(val) = options.ruby_package.as_ref() {
                opts.push(format!(
                    "option ruby_package = \"{}\";",
                    Self::escape_string(val)
                ));
            }

            // Sort options for determinism
            opts.sort();

            for opt in &opts {
                writeln!(self.output, "{opt}")?;
            }

            if !opts.is_empty() {
                self.write_newline();
            }
        }

        Ok(())
    }

    // ========== Messages ==========

    fn write_messages(&mut self, file: &FileDescriptorProto, syntax: &str) -> Result<()> {
        let mut messages = file.message_type.clone();

        if self.options.sort_messages {
            messages.sort_by(|a, b| a.name().cmp(b.name()));
        }

        for message in messages.iter() {
            self.write_message(message, syntax)?;
            self.write_newline();
        }

        Ok(())
    }

    fn write_message(&mut self, message: &DescriptorProto, syntax: &str) -> Result<()> {
        // Skip map entry messages (they're synthetic)
        if self.is_map_entry(message) {
            return Ok(());
        }

        self.write_indent();
        writeln!(self.output, "message {} {{", message.name())?;
        self.indent();

        // Message options
        self.write_message_options(message)?;

        // Nested enums
        for nested_enum in message.enum_type.iter() {
            self.write_enum(nested_enum)?;
        }

        // Nested messages (skip group-generated messages)
        let group_messages = self.get_group_message_names(message);
        for nested_msg in message.nested_type.iter() {
            // Skip messages that are generated from groups
            if !group_messages.contains(nested_msg.name()) {
                self.write_message(nested_msg, syntax)?;
            }
        }

        // Regular fields (non-oneof)
        // Store current message for map field detection
        let saved_message = self.current_message.take();
        self.current_message = Some(message.clone());

        let mut regular_fields: Vec<_> = message
            .field
            .iter()
            // Treat proto3 optional fields as regular fields
            .filter(|f| f.oneof_index.is_none() || f.proto3_optional.unwrap_or(false))
            .collect();

        // Sort by field number for determinism
        regular_fields.sort_by_key(|f| f.number());

        for field in regular_fields {
            self.write_field(field, syntax)?;
        }

        // Oneofs (collect oneof fields)
        let mut oneof_fields: Vec<Vec<&FieldDescriptorProto>> =
            vec![Vec::new(); message.oneof_decl.len()];

        for field in message.field.iter() {
            if let Some(idx) = field.oneof_index {
                // Skip synthetic oneof for proto3 optional fields
                if field.proto3_optional.unwrap_or(false) {
                    continue;
                }
                if (idx as usize) < oneof_fields.len() {
                    oneof_fields[idx as usize].push(field);
                }
            }
        }

        // Write oneofs
        for (idx, oneof) in message.oneof_decl.iter().enumerate() {
            if !oneof_fields[idx].is_empty() {
                self.write_oneof(oneof, &oneof_fields[idx], syntax)?;
            }
        }

        // Restore previous message
        self.current_message = saved_message;

        // Extensions
        for extension in message.extension.iter() {
            self.write_field(extension, syntax)?;
        }

        // Extension ranges
        for range in message.extension_range.iter() {
            self.write_indent();
            if range.start() + 1 == range.end() {
                writeln!(self.output, "extensions {};", range.start())?;
            } else {
                // Max field number is 536870911 (0x1FFFFFFF), stored as end=536870912
                let end_val = range.end() - 1;
                if end_val == 536870911 {
                    writeln!(self.output, "extensions {} to max;", range.start())?;
                } else {
                    writeln!(self.output, "extensions {} to {};", range.start(), end_val)?;
                }
            }
        }

        // Reserved
        self.write_reserved(message)?;

        self.dedent();
        self.write_indent();
        writeln!(self.output, "}}")?;

        Ok(())
    }

    fn is_map_entry(&self, message: &DescriptorProto) -> bool {
        message
            .options
            .as_ref()
            .and_then(|o| o.map_entry)
            .unwrap_or(false)
    }

    fn get_group_message_names(
        &self,
        message: &DescriptorProto,
    ) -> std::collections::HashSet<String> {
        let mut group_names = std::collections::HashSet::new();
        for field in &message.field {
            if let Some(type_) = field.type_ {
                if type_.value() == Type::TYPE_GROUP as i32 {
                    if let Some(type_name) = field.type_name.as_ref() {
                        let group_name = type_name.split('.').next_back().unwrap_or(type_name);
                        group_names.insert(group_name.to_string());
                    }
                }
            }
        }
        group_names
    }

    fn get_map_field_info(&self, field: &FieldDescriptorProto) -> Option<MapFieldInfo> {
        // Check if field is repeated and of message type
        if let Some(label) = field.label {
            if label.value() != Label::LABEL_REPEATED as i32 {
                return None;
            }
        } else {
            return None;
        }

        if let Some(type_) = field.type_ {
            if type_.value() != Type::TYPE_MESSAGE as i32 {
                return None;
            }
        } else {
            return None;
        }

        // Get the message type name and check if it's a nested message
        let type_name = field.type_name.as_ref()?;

        // Extract the entry message name (last component after last dot)
        let entry_name = type_name.split('.').next_back()?;

        // Look for the map entry message in current message's nested types
        let current_msg = self.current_message.as_ref()?;
        let entry_msg = current_msg
            .nested_type
            .iter()
            .find(|m| m.name() == entry_name)?;

        // Check if it's a map entry
        if !self.is_map_entry(entry_msg) {
            return None;
        }

        // Map entry should have exactly 2 fields: key and value
        if entry_msg.field.len() != 2 {
            return None;
        }

        let key_field = entry_msg.field.iter().find(|f| f.name() == "key")?;
        let value_field = entry_msg.field.iter().find(|f| f.name() == "value")?;

        let key_type = if let Some(kt) = key_field.type_ {
            let kt_val = kt.value();
            if kt_val == Type::TYPE_MESSAGE as i32 || kt_val == Type::TYPE_ENUM as i32 {
                self.format_type_name(key_field.type_name.as_ref()?)
            } else {
                self.field_type_to_string(kt).to_string()
            }
        } else {
            return None;
        };

        let value_type = if let Some(vt) = value_field.type_ {
            let vt_val = vt.value();
            if vt_val == Type::TYPE_MESSAGE as i32 || vt_val == Type::TYPE_ENUM as i32 {
                self.format_type_name(value_field.type_name.as_ref()?)
            } else {
                self.field_type_to_string(vt).to_string()
            }
        } else {
            return None;
        };

        Some(MapFieldInfo {
            key_type,
            value_type,
        })
    }

    fn write_message_options(&mut self, message: &DescriptorProto) -> Result<()> {
        if let Some(options) = message.options.as_ref() {
            if let Some(val) = options.message_set_wire_format {
                if val {
                    self.write_indent();
                    writeln!(self.output, "option message_set_wire_format = true;")?;
                }
            }
            if let Some(val) = options.no_standard_descriptor_accessor {
                if val {
                    self.write_indent();
                    writeln!(
                        self.output,
                        "option no_standard_descriptor_accessor = true;"
                    )?;
                }
            }
            if let Some(val) = options.deprecated {
                if val {
                    self.write_indent();
                    writeln!(self.output, "option deprecated = true;")?;
                }
            }
        }
        Ok(())
    }

    fn write_reserved(&mut self, message: &DescriptorProto) -> Result<()> {
        // Reserved ranges (end is exclusive for messages)
        if !message.reserved_range.is_empty() {
            self.write_indent();
            write!(self.output, "reserved ")?;
            for (i, range) in message.reserved_range.iter().enumerate() {
                if i > 0 {
                    write!(self.output, ", ")?;
                }
                if range.start() + 1 == range.end() {
                    write!(self.output, "{}", range.start())?;
                } else {
                    let end_val = range.end() - 1;
                    if end_val == 536870911 {
                        write!(self.output, "{} to max", range.start())?;
                    } else {
                        write!(self.output, "{} to {}", range.start(), end_val)?;
                    }
                }
            }
            writeln!(self.output, ";")?;
        }

        // Reserved names
        if !message.reserved_name.is_empty() {
            self.write_indent();
            write!(self.output, "reserved ")?;
            for (i, name) in message.reserved_name.iter().enumerate() {
                if i > 0 {
                    write!(self.output, ", ")?;
                }
                write!(self.output, "\"{name}\"")?;
            }
            writeln!(self.output, ";")?;
        }

        Ok(())
    }

    // ========== Fields ==========

    fn write_field(&mut self, field: &FieldDescriptorProto, syntax: &str) -> Result<()> {
        // Check if this is a map field
        if let Some(map_info) = self.get_map_field_info(field) {
            self.write_indent();
            write!(
                self.output,
                "map<{}, {}> {} = {}",
                map_info.key_type,
                map_info.value_type,
                field.name(),
                field.number()
            )?;
            self.write_field_options(field)?;
            writeln!(self.output, ";")?;
            return Ok(());
        }

        self.write_indent();

        // Label (required/optional/repeated)
        if let Some(label) = field.label {
            if label.value() == Label::LABEL_REPEATED as i32 {
                write!(self.output, "repeated ")?;
            } else if label.value() == Label::LABEL_REQUIRED as i32 && syntax == "proto2" {
                write!(self.output, "required ")?;
            } else if label.value() == Label::LABEL_OPTIONAL as i32 {
                // In proto2, optional is explicit. In proto3, optional is only printed when proto3_optional is true.
                if syntax == "proto2" || field.proto3_optional.unwrap_or(false) {
                    write!(self.output, "optional ")?;
                }
            }
        }

        // Type
        if let Some(type_) = field.type_ {
            let type_val = type_.value();
            if type_val == Type::TYPE_GROUP as i32 {
                // Groups are special - use type_name (capitalized) instead of field name
                write!(self.output, "group ")?;
                if let Some(type_name) = field.type_name.as_ref() {
                    let group_name = type_name.split('.').next_back().unwrap_or(type_name);
                    write!(self.output, "{group_name}")?;
                } else {
                    write!(self.output, "{}", field.name())?;
                }
                write!(self.output, " = {}", field.number())?;
                self.write_field_options(field)?;
                writeln!(self.output, " {{")?;

                // Find and render group fields (from nested message)
                let group_fields = if let Some(current_msg) = self.current_message.as_ref() {
                    if let Some(type_name) = field.type_name.as_ref() {
                        let group_name = type_name.split('.').next_back().unwrap_or(type_name);
                        current_msg
                            .nested_type
                            .iter()
                            .find(|m| m.name() == group_name)
                            .map(|m| m.field.clone())
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(fields) = group_fields {
                    self.indent();
                    for group_field in &fields {
                        self.write_field(group_field, syntax)?;
                    }
                    self.dedent();
                }

                self.write_indent();
                writeln!(self.output, "}}")?;
                return Ok(());
            } else if type_val == Type::TYPE_MESSAGE as i32 || type_val == Type::TYPE_ENUM as i32 {
                // Use type_name for messages and enums
                if let Some(type_name) = field.type_name.as_ref() {
                    let type_name = self.format_type_name(type_name);
                    write!(self.output, "{type_name} ")?;
                }
            } else {
                write!(self.output, "{} ", self.field_type_to_string(type_))?;
            }
        }

        // Field name
        write!(self.output, "{}", field.name())?;

        // Field number
        write!(self.output, " = {}", field.number())?;

        // Field options
        self.write_field_options(field)?;

        writeln!(self.output, ";")?;

        Ok(())
    }

    fn write_field_options(&mut self, field: &FieldDescriptorProto) -> Result<()> {
        if let Some(options) = field.options.as_ref() {
            let mut opts = Vec::new();

            if let Some(val) = options.packed {
                opts.push(format!("packed = {val}"));
            }
            if let Some(val) = options.deprecated {
                if val {
                    opts.push("deprecated = true".to_string());
                }
            }
            if let Some(val) = options.lazy {
                if val {
                    opts.push("lazy = true".to_string());
                }
            }
            if let Some(val) = options.weak {
                if val {
                    opts.push("weak = true".to_string());
                }
            }
            if let Some(val) = options.ctype {
                let s = match val.value() {
                    x if x == protobuf::descriptor::field_options::CType::STRING as i32 => "STRING",
                    x if x == protobuf::descriptor::field_options::CType::CORD as i32 => "CORD",
                    x if x == protobuf::descriptor::field_options::CType::STRING_PIECE as i32 => {
                        "STRING_PIECE"
                    }
                    _ => "STRING",
                };
                opts.push(format!("ctype = {s}"));
            }
            if let Some(val) = options.jstype {
                let s = match val.value() {
                    x if x == protobuf::descriptor::field_options::JSType::JS_NORMAL as i32 => {
                        "JS_NORMAL"
                    }
                    x if x == protobuf::descriptor::field_options::JSType::JS_STRING as i32 => {
                        "JS_STRING"
                    }
                    x if x == protobuf::descriptor::field_options::JSType::JS_NUMBER as i32 => {
                        "JS_NUMBER"
                    }
                    _ => "JS_NORMAL",
                };
                opts.push(format!("jstype = {s}"));
            }
            if let Some(ref val) = field.default_value {
                // Format default value based on type
                if let Some(type_) = field.type_ {
                    let type_val = type_.value();
                    if type_val == Type::TYPE_STRING as i32 {
                        opts.push(format!("default = \"{}\"", Self::escape_string(val)));
                    } else if type_val == Type::TYPE_BYTES as i32 {
                        // For bytes, escape non-printable and non-ASCII using \xNN
                        let escaped = Self::escape_bytes(val.as_bytes());
                        opts.push(format!("default = \"{escaped}\""));
                    } else if type_val == Type::TYPE_ENUM as i32 {
                        // Enum default: print symbol name. If numeric, map to symbol.
                        let printed = if let Ok(num) = val.parse::<i32>() {
                            if let Some(ref type_name) = field.type_name {
                                self.enum_number_to_name(type_name, num)
                                    .unwrap_or_else(|| val.clone())
                            } else {
                                val.clone()
                            }
                        } else {
                            val.clone()
                        };
                        opts.push(format!("default = {printed}"));
                    } else if type_val == Type::TYPE_FLOAT as i32
                        || type_val == Type::TYPE_DOUBLE as i32
                    {
                        let norm = Self::normalize_float_default(val);
                        opts.push(format!("default = {norm}"));
                    } else {
                        // numeric, bool default values appear as is
                        opts.push(format!("default = {val}"));
                    }
                }
            }

            if !opts.is_empty() {
                write!(self.output, " [")?;
                for (i, opt) in opts.iter().enumerate() {
                    if i > 0 {
                        write!(self.output, ", ")?;
                    }
                    write!(self.output, "{opt}")?;
                }
                write!(self.output, "]")?;
            }
        }

        Ok(())
    }

    fn field_type_to_string(&self, type_: protobuf::EnumOrUnknown<Type>) -> &'static str {
        let val = type_.value();
        if val == Type::TYPE_DOUBLE as i32 {
            "double"
        } else if val == Type::TYPE_FLOAT as i32 {
            "float"
        } else if val == Type::TYPE_INT64 as i32 {
            "int64"
        } else if val == Type::TYPE_UINT64 as i32 {
            "uint64"
        } else if val == Type::TYPE_INT32 as i32 {
            "int32"
        } else if val == Type::TYPE_FIXED64 as i32 {
            "fixed64"
        } else if val == Type::TYPE_FIXED32 as i32 {
            "fixed32"
        } else if val == Type::TYPE_BOOL as i32 {
            "bool"
        } else if val == Type::TYPE_STRING as i32 {
            "string"
        } else if val == Type::TYPE_GROUP as i32 {
            "group"
        } else if val == Type::TYPE_MESSAGE as i32 {
            "message"
        } else if val == Type::TYPE_BYTES as i32 {
            "bytes"
        } else if val == Type::TYPE_UINT32 as i32 {
            "uint32"
        } else if val == Type::TYPE_ENUM as i32 {
            "enum"
        } else if val == Type::TYPE_SFIXED32 as i32 {
            "sfixed32"
        } else if val == Type::TYPE_SFIXED64 as i32 {
            "sfixed64"
        } else if val == Type::TYPE_SINT32 as i32 {
            "sint32"
        } else if val == Type::TYPE_SINT64 as i32 {
            "sint64"
        } else {
            "unknown"
        }
    }

    fn format_type_name(&self, type_name: &str) -> String {
        // Remove leading dot if present
        type_name.trim_start_matches('.').to_string()
    }

    // Resolve enum value name by numeric number for a fully-qualified enum type name.
    fn enum_number_to_name(&self, fq_type_name: &str, number: i32) -> Option<String> {
        let file = self.current_file.as_ref()?;
        let pkg = file.package.as_deref().unwrap_or("");
        let comps: Vec<&str> = fq_type_name.trim_start_matches('.').split('.').collect();
        let pkg_len = if pkg.is_empty() {
            0
        } else {
            pkg.split('.').count()
        };
        if pkg_len > comps.len() {
            return None;
        }
        // Search starting at top-level
        // Check top-level enums
        if pkg_len < comps.len() {
            let first = comps[pkg_len];
            for en in &file.enum_type {
                if en.name() == first {
                    // If this is the last component, we found the enum
                    if pkg_len == comps.len() - 1 {
                        for v in &en.value {
                            if v.number() == number {
                                return Some(v.name().to_string());
                            }
                        }
                    }
                    return None;
                }
            }
        }
        // Traverse nested messages
        let mut idx = pkg_len;
        let mut current_messages: &Vec<DescriptorProto> = &file.message_type;
        let mut current_msg: Option<&DescriptorProto> = None;
        while idx < comps.len() {
            let name = comps[idx];
            // try enum here if this is last
            if idx == comps.len() - 1 {
                if let Some(msg) = current_msg {
                    for en in &msg.enum_type {
                        if en.name() == name {
                            for v in &en.value {
                                if v.number() == number {
                                    return Some(v.name().to_string());
                                }
                            }
                            return None;
                        }
                    }
                }
                // also check top-level enums by this name (just in case)
                for en in &file.enum_type {
                    if en.name() == name {
                        for v in &en.value {
                            if v.number() == number {
                                return Some(v.name().to_string());
                            }
                        }
                        return None;
                    }
                }
                return None;
            }
            // descend into message by this name
            let mut found: Option<&DescriptorProto> = None;
            for m in current_messages {
                if m.name() == name {
                    found = Some(m);
                    break;
                }
            }
            let m = found?;
            current_msg = Some(m);
            current_messages = &m.nested_type;
            idx += 1;
        }
        None
    }

    fn normalize_float_default(val: &str) -> &str {
        match val {
            "Infinity" | "+Infinity" | "+Inf" | "Inf" => "inf",
            "-Infinity" | "-Inf" => "-inf",
            "NaN" | "nan" => "nan",
            _ => val,
        }
    }

    // ========== Oneofs ==========

    fn write_oneof(
        &mut self,
        oneof: &OneofDescriptorProto,
        fields: &[&FieldDescriptorProto],
        syntax: &str,
    ) -> Result<()> {
        self.write_indent();
        writeln!(self.output, "oneof {} {{", oneof.name())?;
        self.indent();

        let mut sorted_fields = fields.to_vec();
        sorted_fields.sort_by_key(|f| f.number());

        for field in sorted_fields {
            self.write_field(field, syntax)?;
        }

        self.dedent();
        self.write_indent();
        writeln!(self.output, "}}")?;

        Ok(())
    }

    // ========== Enums ==========

    fn write_enums(&mut self, file: &FileDescriptorProto) -> Result<()> {
        let mut enums = file.enum_type.clone();

        if self.options.sort_enums {
            enums.sort_by(|a, b| a.name().cmp(b.name()));
        }

        for enum_type in enums.iter() {
            self.write_enum(enum_type)?;
            self.write_newline();
        }

        Ok(())
    }

    fn write_enum(&mut self, enum_type: &EnumDescriptorProto) -> Result<()> {
        self.write_indent();
        writeln!(self.output, "enum {} {{", enum_type.name())?;
        self.indent();

        // Enum options
        self.write_enum_options(enum_type)?;

        // Enum values - sorted by number for determinism
        let mut values = enum_type.value.clone();
        values.sort_by_key(|v| v.number());

        for value in values.iter() {
            self.write_enum_value(value)?;
        }

        // Reserved
        self.write_enum_reserved(enum_type)?;

        self.dedent();
        self.write_indent();
        writeln!(self.output, "}}")?;

        Ok(())
    }

    fn write_enum_options(&mut self, enum_type: &EnumDescriptorProto) -> Result<()> {
        if let Some(options) = enum_type.options.as_ref() {
            if let Some(val) = options.allow_alias {
                if val {
                    self.write_indent();
                    writeln!(self.output, "option allow_alias = true;")?;
                }
            }
            if let Some(val) = options.deprecated {
                if val {
                    self.write_indent();
                    writeln!(self.output, "option deprecated = true;")?;
                }
            }
        }
        Ok(())
    }

    fn write_enum_value(&mut self, value: &EnumValueDescriptorProto) -> Result<()> {
        self.write_indent();
        write!(self.output, "{} = {}", value.name(), value.number())?;

        // Value options
        if let Some(options) = value.options.as_ref() {
            if let Some(val) = options.deprecated {
                if val {
                    write!(self.output, " [deprecated = true]")?;
                }
            }
        }

        writeln!(self.output, ";")?;
        Ok(())
    }

    fn write_enum_reserved(&mut self, enum_type: &EnumDescriptorProto) -> Result<()> {
        // Reserved ranges (end is inclusive for enums)
        if !enum_type.reserved_range.is_empty() {
            self.write_indent();
            write!(self.output, "reserved ")?;
            for (i, range) in enum_type.reserved_range.iter().enumerate() {
                if i > 0 {
                    write!(self.output, ", ")?;
                }
                if range.start() == range.end() {
                    write!(self.output, "{}", range.start())?;
                } else {
                    // For enums, max field number is also 536870911
                    if range.end() == 536870911 {
                        write!(self.output, "{} to max", range.start())?;
                    } else {
                        write!(self.output, "{} to {}", range.start(), range.end())?;
                    }
                }
            }
            writeln!(self.output, ";")?;
        }

        // Reserved names
        if !enum_type.reserved_name.is_empty() {
            self.write_indent();
            write!(self.output, "reserved ")?;
            for (i, name) in enum_type.reserved_name.iter().enumerate() {
                if i > 0 {
                    write!(self.output, ", ")?;
                }
                write!(self.output, "\"{name}\"")?;
            }
            writeln!(self.output, ";")?;
        }

        Ok(())
    }

    // ========== Services ==========

    fn write_services(&mut self, file: &FileDescriptorProto) -> Result<()> {
        let mut services = file.service.clone();

        if self.options.sort_services {
            services.sort_by(|a, b| a.name().cmp(b.name()));
        }

        for service in services.iter() {
            self.write_service(service)?;
            self.write_newline();
        }

        Ok(())
    }

    fn write_service(&mut self, service: &ServiceDescriptorProto) -> Result<()> {
        self.write_indent();
        writeln!(self.output, "service {} {{", service.name())?;
        self.indent();

        // Service options
        self.write_service_options(service)?;

        // Methods - sorted by name for determinism
        let mut methods = service.method.clone();
        methods.sort_by(|a, b| a.name().cmp(b.name()));

        for method in methods.iter() {
            self.write_method(method)?;
        }

        self.dedent();
        self.write_indent();
        writeln!(self.output, "}}")?;

        Ok(())
    }

    fn write_service_options(&mut self, service: &ServiceDescriptorProto) -> Result<()> {
        if let Some(options) = service.options.as_ref() {
            if let Some(val) = options.deprecated {
                if val {
                    self.write_indent();
                    writeln!(self.output, "option deprecated = true;")?;
                }
            }
        }
        Ok(())
    }

    fn write_method(&mut self, method: &MethodDescriptorProto) -> Result<()> {
        self.write_indent();
        write!(self.output, "rpc {}", method.name())?;

        // Input type
        write!(self.output, "(")?;
        if method.client_streaming() {
            write!(self.output, "stream ")?;
        }
        if let Some(input_type) = method.input_type.as_ref() {
            write!(self.output, "{}", self.format_type_name(input_type))?;
        }
        write!(self.output, ")")?;

        // Return type
        write!(self.output, " returns (")?;
        if method.server_streaming() {
            write!(self.output, "stream ")?;
        }
        if let Some(output_type) = method.output_type.as_ref() {
            write!(self.output, "{}", self.format_type_name(output_type))?;
        }
        write!(self.output, ")")?;

        // Method options
        if let Some(options) = method.options.as_ref() {
            if let Some(val) = options.deprecated {
                if val {
                    write!(self.output, " {{")?;
                    self.write_newline();
                    self.indent();
                    self.write_indent();
                    writeln!(self.output, "option deprecated = true;")?;
                    self.dedent();
                    self.write_indent();
                    write!(self.output, "}}")?;
                }
            }
        }

        writeln!(self.output, ";")?;
        Ok(())
    }

    // ========== Extensions ==========

    fn write_extensions(&mut self, file: &FileDescriptorProto, syntax: &str) -> Result<()> {
        use std::collections::BTreeMap;
        // Group by extendee
        let mut groups: BTreeMap<String, Vec<&FieldDescriptorProto>> = BTreeMap::new();
        for ext in &file.extension {
            let key = ext.extendee.clone().unwrap_or_default();
            groups.entry(key).or_default().push(ext);
        }
        for (extendee, mut fields) in groups {
            // Skip empty extendee
            let extendee_fmt = self.format_type_name(&extendee);
            self.write_indent();
            writeln!(self.output, "extend {extendee_fmt} {{")?;
            self.indent();
            // Sort by number for determinism
            fields.sort_by_key(|f| f.number());
            for f in fields {
                self.write_field(f, syntax)?;
            }
            self.dedent();
            self.write_indent();
            writeln!(self.output, "}}")?;
        }
        Ok(())
    }
}

/// Convenience function to convert a FileDescriptorProto to proto text.
pub fn descriptor_to_proto(file: &FileDescriptorProto) -> Result<String> {
    let mut generator = TextGenerator::with_default();
    generator.format_file(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_constant() {
        assert_eq!(TEXT_GENERATOR_VERSION, "1.0.0");
    }

    #[test]
    fn test_field_type_mapping() {
        let generator = TextGenerator::with_default();
        assert_eq!(
            generator.field_type_to_string(Type::TYPE_INT32.into()),
            "int32"
        );
        assert_eq!(
            generator.field_type_to_string(Type::TYPE_STRING.into()),
            "string"
        );
        assert_eq!(
            generator.field_type_to_string(Type::TYPE_BOOL.into()),
            "bool"
        );
        assert_eq!(
            generator.field_type_to_string(Type::TYPE_DOUBLE.into()),
            "double"
        );
    }

    #[test]
    fn test_format_type_name() {
        let generator = TextGenerator::with_default();
        assert_eq!(generator.format_type_name(".foo.Bar"), "foo.Bar");
        assert_eq!(generator.format_type_name("foo.Bar"), "foo.Bar");
        assert_eq!(generator.format_type_name(".Bar"), "Bar");
    }
}
