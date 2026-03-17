use rusqlite::{params, CachedStatement, Transaction};

use super::types::FileIndexData;

pub(crate) struct SnapshotWriter<'tx> {
    tx: &'tx Transaction<'tx>,
    insert_file: CachedStatement<'tx>,
    insert_function: CachedStatement<'tx>,
    insert_function_fts: CachedStatement<'tx>,
    insert_function_param: CachedStatement<'tx>,
    insert_class: CachedStatement<'tx>,
    insert_class_fts: CachedStatement<'tx>,
    insert_class_method: CachedStatement<'tx>,
    insert_class_super: CachedStatement<'tx>,
    insert_field: CachedStatement<'tx>,
    insert_call: CachedStatement<'tx>,
    insert_import: CachedStatement<'tx>,
    insert_import_fts: CachedStatement<'tx>,
    insert_annotation: CachedStatement<'tx>,
    insert_annotation_fts: CachedStatement<'tx>,
    insert_ref: CachedStatement<'tx>,
    insert_python_property: CachedStatement<'tx>,
    insert_python_property_caller: CachedStatement<'tx>,
}

impl<'tx> SnapshotWriter<'tx> {
    pub(crate) fn new(tx: &'tx Transaction<'tx>) -> anyhow::Result<Self> {
        Ok(Self {
            tx,
            insert_file: tx.prepare_cached(
                "INSERT INTO files(path, language, mtime_nanos, size_bytes, content_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?,
            insert_function: tx.prepare_cached(
                "
                INSERT INTO functions(file_id, name, class_name, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
            )?,
            insert_function_fts: tx.prepare_cached(
                "INSERT INTO functions_fts(rowid, name) VALUES (?1, ?2)",
            )?,
            insert_function_param: tx.prepare_cached(
                "
                INSERT INTO function_params(function_id, name, param_type, position)
                VALUES (?1, ?2, ?3, ?4)
                ",
            )?,
            insert_class: tx.prepare_cached(
                "
                INSERT INTO classes(file_id, name, kind, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
            )?,
            insert_class_fts: tx.prepare_cached(
                "INSERT INTO classes_fts(rowid, name) VALUES (?1, ?2)",
            )?,
            insert_class_method: tx.prepare_cached(
                "INSERT INTO class_methods(class_id, method_name) VALUES (?1, ?2)",
            )?,
            insert_class_super: tx.prepare_cached(
                "INSERT INTO class_super_classes(class_id, super_class_name) VALUES (?1, ?2)",
            )?,
            insert_field: tx.prepare_cached(
                "
                INSERT INTO fields(file_id, class_name, name, field_type, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ",
            )?,
            insert_call: tx.prepare_cached(
                "
                INSERT INTO calls(file_id, callee, caller, caller_class_name, object_name, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ",
            )?,
            insert_import: tx.prepare_cached(
                "INSERT INTO imports(file_id, module, start_line, end_line) VALUES (?1, ?2, ?3, ?4)",
            )?,
            insert_import_fts: tx.prepare_cached(
                "INSERT INTO imports_fts(rowid, module) VALUES (?1, ?2)",
            )?,
            insert_annotation: tx.prepare_cached(
                "
                INSERT INTO annotations(file_id, name, signature, start_line, end_line, target_name, target_type, target_signature)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ",
            )?,
            insert_annotation_fts: tx.prepare_cached(
                "INSERT INTO annotations_fts(rowid, name) VALUES (?1, ?2)",
            )?,
            insert_ref: tx.prepare_cached(
                "
                INSERT INTO refs(file_id, name, node_type, start_line, end_line, start_column, end_column)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ",
            )?,
            insert_python_property: tx.prepare_cached(
                "INSERT INTO python_properties(file_id, property_name, class_name) VALUES (?1, ?2, ?3)",
            )?,
            insert_python_property_caller: tx.prepare_cached(
                "
                INSERT INTO python_property_callers(file_id, property_name, caller, caller_class_name, object_name, object_type, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ",
            )?,
        })
    }

    pub(crate) fn insert_snapshot(&mut self, snapshot: &FileIndexData) -> anyhow::Result<()> {
        self.insert_file.execute(params![
            snapshot.file.path,
            snapshot.file.language,
            snapshot.file.mtime_nanos,
            snapshot.file.size_bytes,
            snapshot.file.content_hash,
        ])?;
        let file_id = self.tx.last_insert_rowid();

        for function in &snapshot.snapshot.functions {
            self.insert_function.execute(params![
                file_id,
                function.name,
                function.class_name,
                function.location.start_line as i64,
                function.location.end_line as i64
            ])?;
            let function_id = self.tx.last_insert_rowid();
            self.insert_function_fts
                .execute(params![function_id, function.name])?;
            for (position, param) in function.params.iter().enumerate() {
                self.insert_function_param.execute(params![
                    function_id,
                    param.name,
                    param.param_type,
                    position as i64
                ])?;
            }
        }

        for class in &snapshot.snapshot.classes {
            self.insert_class.execute(params![
                file_id,
                class.name,
                class.kind,
                class.location.start_line as i64,
                class.location.end_line as i64
            ])?;
            let class_id = self.tx.last_insert_rowid();
            self.insert_class_fts
                .execute(params![class_id, class.name])?;
            for method in &class.methods {
                self.insert_class_method
                    .execute(params![class_id, method])?;
            }
            for super_class in &class.super_classes {
                self.insert_class_super
                    .execute(params![class_id, super_class])?;
            }
        }

        for field in &snapshot.snapshot.fields {
            self.insert_field.execute(params![
                file_id,
                field.class_name,
                field.name,
                field.field_type,
                field.location.start_line as i64,
                field.location.end_line as i64
            ])?;
        }

        for call in &snapshot.snapshot.calls {
            self.insert_call.execute(params![
                file_id,
                call.callee,
                call.caller,
                call.caller_class_name,
                call.object_name,
                call.location.start_line as i64,
                call.location.end_line as i64
            ])?;
        }

        for import in &snapshot.snapshot.imports {
            self.insert_import.execute(params![
                file_id,
                import.module,
                import.location.start_line as i64,
                import.location.end_line as i64
            ])?;
            let import_id = self.tx.last_insert_rowid();
            self.insert_import_fts
                .execute(params![import_id, import.module])?;
        }

        for annotation in &snapshot.snapshot.annotations {
            self.insert_annotation.execute(params![
                file_id,
                annotation.name,
                annotation.signature,
                annotation.location.start_line as i64,
                annotation.location.end_line as i64,
                annotation.target_name,
                annotation.target_type,
                annotation.target_signature
            ])?;
            let annotation_id = self.tx.last_insert_rowid();
            self.insert_annotation_fts
                .execute(params![annotation_id, annotation.name])?;
        }

        for reference in &snapshot.snapshot.refs {
            self.insert_ref.execute(params![
                file_id,
                reference.name,
                reference.node_type,
                reference.location.start_line as i64,
                reference.location.end_line as i64,
                reference.start_column as i64,
                reference.end_column as i64
            ])?;
        }

        for property in &snapshot.snapshot.python_properties {
            self.insert_python_property.execute(params![
                file_id,
                property.name,
                property.class_name
            ])?;
        }

        for caller in &snapshot.snapshot.python_property_callers {
            self.insert_python_property_caller.execute(params![
                file_id,
                caller.property_name,
                caller.caller,
                caller.caller_class_name,
                caller.object_name,
                caller.object_type,
                caller.location.start_line as i64,
                caller.location.end_line as i64
            ])?;
        }

        Ok(())
    }
}
