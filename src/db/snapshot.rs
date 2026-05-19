use rusqlite::{params, CachedStatement, Transaction};

use super::types::IndexedFileSnapshot;

pub(crate) struct SnapshotWriter<'tx> {
    tx: &'tx Transaction<'tx>,
    maintain_fts: bool,
    insert_file: CachedStatement<'tx>,
    insert_function: CachedStatement<'tx>,
    insert_function_fts: Option<CachedStatement<'tx>>,
    insert_function_param: CachedStatement<'tx>,
    insert_class: CachedStatement<'tx>,
    insert_class_fts: Option<CachedStatement<'tx>>,
    insert_class_method: CachedStatement<'tx>,
    insert_class_super: CachedStatement<'tx>,
    insert_field: CachedStatement<'tx>,
    insert_call: CachedStatement<'tx>,
    insert_import: CachedStatement<'tx>,
    insert_import_fts: Option<CachedStatement<'tx>>,
    insert_annotation: CachedStatement<'tx>,
    insert_annotation_fts: Option<CachedStatement<'tx>>,
    insert_ref: CachedStatement<'tx>,
    insert_python_property: CachedStatement<'tx>,
    insert_python_property_caller: CachedStatement<'tx>,
}

impl<'tx> SnapshotWriter<'tx> {
    pub(crate) fn new(tx: &'tx Transaction<'tx>) -> anyhow::Result<Self> {
        Self::with_fts(tx, true)
    }

    pub(crate) fn without_fts(tx: &'tx Transaction<'tx>) -> anyhow::Result<Self> {
        Self::with_fts(tx, false)
    }

    fn with_fts(tx: &'tx Transaction<'tx>, maintain_fts: bool) -> anyhow::Result<Self> {
        Ok(Self {
            tx,
            maintain_fts,
            insert_file: tx.prepare_cached(
                "INSERT INTO files(path, language, mtime_nanos, size_bytes, content_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?,
            insert_function: tx.prepare_cached(
                "
                INSERT INTO functions(file_id, name, class_name, start_line, end_line)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
            )?,
            insert_function_fts: maintain_fts
                .then(|| tx.prepare_cached("INSERT INTO functions_fts(rowid, name) VALUES (?1, ?2)"))
                .transpose()?,
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
            insert_class_fts: maintain_fts
                .then(|| tx.prepare_cached("INSERT INTO classes_fts(rowid, name) VALUES (?1, ?2)"))
                .transpose()?,
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
            insert_import_fts: maintain_fts
                .then(|| tx.prepare_cached("INSERT INTO imports_fts(rowid, module) VALUES (?1, ?2)"))
                .transpose()?,
            insert_annotation: tx.prepare_cached(
                "
                INSERT INTO annotations(file_id, name, signature, start_line, end_line, target_name, target_type, target_signature)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ",
            )?,
            insert_annotation_fts: maintain_fts
                .then(|| {
                    tx.prepare_cached("INSERT INTO annotations_fts(rowid, name) VALUES (?1, ?2)")
                })
                .transpose()?,
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

    pub(crate) fn insert_snapshot(&mut self, snapshot: &IndexedFileSnapshot) -> anyhow::Result<()> {
        self.insert_file.execute(params![
            snapshot.metadata.path,
            snapshot.metadata.language,
            snapshot.metadata.mtime_nanos,
            snapshot.metadata.size_bytes,
            snapshot.metadata.content_hash,
        ])?;
        let file_id = self.tx.last_insert_rowid();

        self.insert_functions(file_id, snapshot)?;
        self.insert_classes(file_id, snapshot)?;
        self.insert_fields(file_id, snapshot)?;
        self.insert_calls(file_id, snapshot)?;
        self.insert_imports(file_id, snapshot)?;
        self.insert_annotations(file_id, snapshot)?;
        self.insert_refs(file_id, snapshot)?;
        self.insert_python_properties(file_id, snapshot)?;
        self.insert_python_property_callers(file_id, snapshot)?;

        Ok(())
    }

    fn insert_functions(
        &mut self,
        file_id: i64,
        snapshot: &IndexedFileSnapshot,
    ) -> anyhow::Result<()> {
        for function in &snapshot.snapshot.functions {
            self.insert_function.execute(params![
                file_id,
                function.name,
                function.class_name,
                function.location.start_line,
                function.location.end_line
            ])?;
            let function_id = self.tx.last_insert_rowid();
            if self.maintain_fts {
                self.insert_function_fts
                    .as_mut()
                    .expect("fts statement should exist when enabled")
                    .execute(params![function_id, function.name])?;
            }
            for (position, param) in function.params.iter().enumerate() {
                self.insert_function_param.execute(params![
                    function_id,
                    param.name,
                    param.param_type,
                    position
                ])?;
            }
        }
        Ok(())
    }

    fn insert_classes(
        &mut self,
        file_id: i64,
        snapshot: &IndexedFileSnapshot,
    ) -> anyhow::Result<()> {
        for class in &snapshot.snapshot.classes {
            self.insert_class.execute(params![
                file_id,
                class.name,
                class.kind,
                class.location.start_line,
                class.location.end_line
            ])?;
            let class_id = self.tx.last_insert_rowid();
            if self.maintain_fts {
                self.insert_class_fts
                    .as_mut()
                    .expect("fts statement should exist when enabled")
                    .execute(params![class_id, class.name])?;
            }
            for method in &class.methods {
                self.insert_class_method
                    .execute(params![class_id, method])?;
            }
            for super_class in &class.super_classes {
                self.insert_class_super
                    .execute(params![class_id, super_class])?;
            }
        }
        Ok(())
    }

    fn insert_fields(
        &mut self,
        file_id: i64,
        snapshot: &IndexedFileSnapshot,
    ) -> anyhow::Result<()> {
        for field in &snapshot.snapshot.fields {
            self.insert_field.execute(params![
                file_id,
                field.class_name,
                field.name,
                field.field_type,
                field.location.start_line,
                field.location.end_line
            ])?;
        }
        Ok(())
    }

    fn insert_calls(&mut self, file_id: i64, snapshot: &IndexedFileSnapshot) -> anyhow::Result<()> {
        for call in &snapshot.snapshot.calls {
            self.insert_call.execute(params![
                file_id,
                call.callee,
                call.caller,
                call.caller_class_name,
                call.object_name,
                call.location.start_line,
                call.location.end_line
            ])?;
        }
        Ok(())
    }

    fn insert_imports(
        &mut self,
        file_id: i64,
        snapshot: &IndexedFileSnapshot,
    ) -> anyhow::Result<()> {
        for import in &snapshot.snapshot.imports {
            self.insert_import.execute(params![
                file_id,
                import.module,
                import.location.start_line,
                import.location.end_line
            ])?;
            let import_id = self.tx.last_insert_rowid();
            if self.maintain_fts {
                self.insert_import_fts
                    .as_mut()
                    .expect("fts statement should exist when enabled")
                    .execute(params![import_id, import.module])?;
            }
        }
        Ok(())
    }

    fn insert_annotations(
        &mut self,
        file_id: i64,
        snapshot: &IndexedFileSnapshot,
    ) -> anyhow::Result<()> {
        for annotation in &snapshot.snapshot.annotations {
            self.insert_annotation.execute(params![
                file_id,
                annotation.name,
                annotation.signature,
                annotation.location.start_line,
                annotation.location.end_line,
                annotation.target_name,
                annotation.target_type,
                annotation.target_signature
            ])?;
            let annotation_id = self.tx.last_insert_rowid();
            if self.maintain_fts {
                self.insert_annotation_fts
                    .as_mut()
                    .expect("fts statement should exist when enabled")
                    .execute(params![annotation_id, annotation.name])?;
            }
        }
        Ok(())
    }

    fn insert_refs(&mut self, file_id: i64, snapshot: &IndexedFileSnapshot) -> anyhow::Result<()> {
        for reference in &snapshot.snapshot.refs {
            self.insert_ref.execute(params![
                file_id,
                reference.name,
                reference.node_type,
                reference.location.start_line,
                reference.location.end_line,
                reference.start_column,
                reference.end_column
            ])?;
        }
        Ok(())
    }

    fn insert_python_properties(
        &mut self,
        file_id: i64,
        snapshot: &IndexedFileSnapshot,
    ) -> anyhow::Result<()> {
        for property in &snapshot.snapshot.python_properties {
            self.insert_python_property.execute(params![
                file_id,
                property.name,
                property.class_name
            ])?;
        }
        Ok(())
    }

    fn insert_python_property_callers(
        &mut self,
        file_id: i64,
        snapshot: &IndexedFileSnapshot,
    ) -> anyhow::Result<()> {
        for caller in &snapshot.snapshot.python_property_callers {
            self.insert_python_property_caller.execute(params![
                file_id,
                caller.property_name,
                caller.caller,
                caller.caller_class_name,
                caller.object_name,
                caller.object_type,
                caller.location.start_line,
                caller.location.end_line
            ])?;
        }
        Ok(())
    }
}
