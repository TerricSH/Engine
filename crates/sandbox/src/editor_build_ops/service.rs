use super::*;

#[derive(Clone, Debug)]
pub(super) enum EditorBuildToolchain {
    Installed(Box<crate::engine_installation::EngineInstallation>),
    Development { repository_root: PathBuf },
}

impl EditorBuildService {
    /// Build a service for a running editor.
    ///
    /// Installed distributions are resolved first. Only a process without an
    /// installation manifest may use the compile-time repository fallback.
    pub(crate) fn for_current_editor() -> Result<Self, EditorBuildError> {
        let operation = EditorBuildOperationKind::Validate;
        let sandbox_executable = std::env::current_exe().map_err(|error| {
            EditorBuildError::request(
                operation,
                EditorBuildFailureKind::InvalidRequest,
                format!("could not locate the running sandbox executable: {error}"),
            )
        })?;
        let installation =
            crate::engine_installation::EngineInstallation::discover_from_current_executable()
                .map_err(|message| {
                    EditorBuildError::request(
                        operation,
                        EditorBuildFailureKind::InvalidRequest,
                        message,
                    )
                })?;
        if let Some(installation) = installation {
            return Self::with_installed_powershell(
                installation,
                system_powershell_executable().map_err(|message| {
                    EditorBuildError::request(
                        operation,
                        EditorBuildFailureKind::InvalidRequest,
                        message,
                    )
                })?,
            );
        }

        let repository_root =
            crate::engine_installation::development_source_root().map_err(|message| {
                EditorBuildError::request(
                    operation,
                    EditorBuildFailureKind::InvalidRequest,
                    message,
                )
            })?;
        Self::with_powershell(
            repository_root,
            sandbox_executable,
            system_powershell_executable().map_err(|message| {
                EditorBuildError::request(
                    operation,
                    EditorBuildFailureKind::InvalidRequest,
                    message,
                )
            })?,
        )
    }

    pub(super) fn with_powershell(
        repository_root: impl Into<PathBuf>,
        sandbox_executable: impl Into<PathBuf>,
        powershell_executable: impl Into<PathBuf>,
    ) -> Result<Self, EditorBuildError> {
        let operation = EditorBuildOperationKind::Validate;
        let repository_root = canonical_directory(repository_root.into(), "repository root")
            .map_err(|message| {
                EditorBuildError::request(
                    operation,
                    EditorBuildFailureKind::InvalidRequest,
                    message,
                )
            })?;
        let sandbox_executable = canonical_file(sandbox_executable.into(), "sandbox executable")
            .map_err(|message| {
                EditorBuildError::request(
                    operation,
                    EditorBuildFailureKind::InvalidRequest,
                    message,
                )
            })?;
        let powershell_executable = powershell_executable.into();
        if powershell_executable.as_os_str().is_empty() {
            return Err(EditorBuildError::request(
                operation,
                EditorBuildFailureKind::InvalidRequest,
                "PowerShell executable must not be empty",
            ));
        }
        Ok(Self {
            toolchain: EditorBuildToolchain::Development { repository_root },
            sandbox_executable,
            powershell_executable,
        })
    }

    pub(super) fn with_installed_powershell(
        installation: crate::engine_installation::EngineInstallation,
        powershell_executable: impl Into<PathBuf>,
    ) -> Result<Self, EditorBuildError> {
        let operation = EditorBuildOperationKind::Validate;
        let powershell_executable = powershell_executable.into();
        if powershell_executable.as_os_str().is_empty() {
            return Err(EditorBuildError::request(
                operation,
                EditorBuildFailureKind::InvalidRequest,
                "PowerShell executable must not be empty",
            ));
        }
        let sandbox_executable = installation.editor.clone();
        Ok(Self {
            toolchain: EditorBuildToolchain::Installed(Box::new(installation)),
            sandbox_executable,
            powershell_executable,
        })
    }

    /// Launch a cancellable background operation.
    pub(crate) fn start(
        &self,
        project_path: impl AsRef<Path>,
        operation: EditorBuildOperation,
    ) -> Result<EditorBuildTask, EditorBuildError> {
        #[cfg(not(windows))]
        let kind = operation.kind();
        #[cfg(not(windows))]
        if matches!(operation, EditorBuildOperation::PackageWindows(_)) {
            return Err(EditorBuildError::request(
                kind,
                EditorBuildFailureKind::UnsupportedPlatform,
                "the Windows player package script can only run on Windows",
            ));
        }

        let plan = self.plan(project_path.as_ref(), operation)?;
        EditorBuildTask::spawn(plan)
    }

    pub(super) fn plan(
        &self,
        project_path: &Path,
        operation: EditorBuildOperation,
    ) -> Result<ProcessPlan, EditorBuildError> {
        let kind = operation.kind();
        let project = GameProject::load(project_path).map_err(|error| {
            EditorBuildError::request(
                kind,
                EditorBuildFailureKind::InvalidRequest,
                format!("project cannot be loaded for authoring: {error}"),
            )
        })?;
        let manifest_path = canonical_file(project.manifest_path.clone(), "project manifest")
            .map_err(|message| {
                EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
            })?;

        match operation {
            EditorBuildOperation::Validate => {
                let report_directory = tempfile::Builder::new()
                    .prefix("engine-editor-project-check-")
                    .tempdir()
                    .map_err(|error| {
                        EditorBuildError::request(
                            kind,
                            EditorBuildFailureKind::InvalidRequest,
                            format!("could not create a validation report directory: {error}"),
                        )
                    })?;
                let report_path = report_directory.path().join("project-check.json");
                Ok(ProcessPlan {
                    kind,
                    executable: self.sandbox_executable.clone(),
                    arguments: vec![
                        OsString::from("project"),
                        OsString::from("check"),
                        manifest_path.clone().into_os_string(),
                        OsString::from("--report"),
                        report_path.clone().into_os_string(),
                    ],
                    working_directory: project.root.clone(),
                    completion: CompletionPlan::Validate {
                        report_path,
                        _report_directory: report_directory,
                    },
                })
            }
            EditorBuildOperation::CookAndCompile => Ok(ProcessPlan {
                kind,
                executable: self.sandbox_executable.clone(),
                arguments: vec![
                    OsString::from("project"),
                    OsString::from("build"),
                    manifest_path.clone().into_os_string(),
                ],
                working_directory: project.root.clone(),
                completion: CompletionPlan::CookAndCompile { manifest_path },
            }),
            EditorBuildOperation::PackageWindows(options) => {
                self.plan_windows_package(&project, manifest_path, options)
            }
        }
    }

    fn plan_windows_package(
        &self,
        project: &GameProject,
        manifest_path: PathBuf,
        options: PackageWindowsOptions,
    ) -> Result<ProcessPlan, EditorBuildError> {
        let kind = EditorBuildOperationKind::PackageWindows;
        validate_release_version(&options.version).map_err(|message| {
            EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
        })?;

        let (script, installation_root) = match &self.toolchain {
            EditorBuildToolchain::Installed(installation) => (
                installation.package_script.clone(),
                Some(installation.root.clone()),
            ),
            EditorBuildToolchain::Development { repository_root } => {
                let script = canonical_file(
                    repository_root.join(".github/scripts/package-windows.ps1"),
                    "development Windows package script",
                )
                .map_err(|message| {
                    EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
                })?;
                if !script.starts_with(repository_root) {
                    return Err(EditorBuildError::request(
                        kind,
                        EditorBuildFailureKind::InvalidRequest,
                        "development Windows package script resolves outside the repository",
                    ));
                }
                (script, None)
            }
        };

        let installed = matches!(&self.toolchain, EditorBuildToolchain::Installed(_));
        let output_root = resolve_output_directory(
            &project.root,
            &options.output_root,
            "package output root",
            installed,
        )
        .map_err(|message| {
            EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
        })?;
        if installed {
            validate_installed_package_output(project, &output_root, &options.version).map_err(
                |message| {
                    EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
                },
            )?;
        }
        let cargo_target_dir = options
            .cargo_target_dir
            .as_deref()
            .map(|path| {
                resolve_output_directory(&project.root, path, "Cargo target directory", false)
            })
            .transpose()
            .map_err(|message| {
                EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
            })?;
        if installation_root.is_some() && cargo_target_dir.is_some() {
            return Err(EditorBuildError::request(
                kind,
                EditorBuildFailureKind::InvalidRequest,
                "an installed engine uses prebuilt tools and does not accept a Cargo target directory",
            ));
        }

        let mut arguments = vec![
            OsString::from("-NoLogo"),
            OsString::from("-NoProfile"),
            OsString::from("-NonInteractive"),
            OsString::from("-ExecutionPolicy"),
            OsString::from("Bypass"),
            OsString::from("-File"),
            script.into_os_string(),
            OsString::from("-ProjectPath"),
            manifest_path.into_os_string(),
            OsString::from("-Version"),
            OsString::from(&options.version),
            OsString::from("-OutputRoot"),
            output_root.clone().into_os_string(),
            OsString::from("-Backend"),
            OsString::from("vulkan"),
        ];
        if let Some(root) = installation_root.as_ref() {
            arguments.push(OsString::from("-EngineInstallRoot"));
            arguments.push(root.clone().into_os_string());
        }
        if let Some(target_dir) = cargo_target_dir {
            arguments.push(OsString::from("-CargoTargetDir"));
            arguments.push(target_dir.into_os_string());
        }
        if options.skip_build {
            arguments.push(OsString::from("-SkipBuild"));
        }
        if options.skip_smoke {
            arguments.push(OsString::from("-SkipSmoke"));
        }
        if options.allow_dirty {
            arguments.push(OsString::from("-AllowDirty"));
        }

        let release_root = output_root.join(&options.version);
        Ok(ProcessPlan {
            kind,
            executable: self.powershell_executable.clone(),
            arguments,
            working_directory: project.root.clone(),
            completion: CompletionPlan::PackageWindows {
                version: options.version,
                allow_dirty: options.allow_dirty,
                release_root,
            },
        })
    }
}
