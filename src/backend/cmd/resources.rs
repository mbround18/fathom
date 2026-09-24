//! Refresh/select/delete commands for network, storage, config, policy,
//! admission and access-control resources.

use crossbeam_channel::Sender;

use crate::backend::state::BackendState;
use crate::backend::{
    applications, clusterrolebindings, clusterroles, configmaps, crds, endpoints, flux, helm,
    horizontalpodautoscalers, ingressclasses, ingresses, leases, limitranges, mutatingwebhooks,
    networkpolicies, persistentvolumeclaims, persistentvolumes, poddisruptionbudgets,
    priorityclasses, rbac_access, related, resourcequotas, rolebindings, roles, runtimeclasses,
    secrets, services, storageclasses, validatingwebhooks,
};
use crate::protocol::{BackendCmd, BackendEventEnvelope};

/// Handles resource commands; returns the command back if it belongs to another family.
pub(super) async fn handle(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    let Some(cmd) = handle_network_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    let Some(cmd) = handle_storage_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    let Some(cmd) = handle_config_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    let Some(cmd) = handle_policy_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    let Some(cmd) = handle_admission_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    let Some(cmd) = handle_access_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    let Some(cmd) = handle_crd_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    let Some(cmd) = handle_helm_cmd(cmd, state, sender).await? else {
        return Ok(None);
    };
    handle_applications_cmd(cmd, state, sender).await
}

/// Helm release read/mutate commands (005 read-only view + 006-helm-mutate rollback/upgrade/
/// uninstall) — split out of `handle_applications_cmd` to keep that function under clippy's
/// `too_many_lines` threshold now that all three 006 user stories' command arms live here.
async fn handle_helm_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshHelmReleases => {
            helm::list_releases(state, sender).await?;
        }
        BackendCmd::SelectHelmRelease { namespace, name } => {
            helm::get_release_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::PreviewHelmRollback {
            namespace,
            release,
            target_revision,
        } => {
            let result =
                helm::preview_helm_rollback(state, &namespace, &release, target_revision).await;
            emit_helm_mutation_preview(state, sender, &namespace, result);
        }
        BackendCmd::ApplyHelmRollback {
            namespace,
            release,
            target_revision,
            base_revision,
        } => {
            let result = helm::apply_helm_rollback(
                state,
                &namespace,
                &release,
                target_revision,
                base_revision,
            )
            .await;
            emit_helm_mutation_applied(state, sender, &namespace, result);
        }
        BackendCmd::UninstallHelmRelease {
            namespace,
            release,
            base_revision,
        } => {
            handle_uninstall_helm_release(state, sender, namespace, release, base_revision).await?;
        }
        BackendCmd::PreviewHelmUpgrade {
            namespace,
            release,
            values_yaml,
            chart_version,
        } => {
            handle_preview_helm_upgrade(
                state,
                sender,
                namespace,
                release,
                values_yaml,
                chart_version,
            )
            .await?;
        }
        BackendCmd::ApplyHelmUpgrade {
            namespace,
            release,
            values_yaml,
            chart_version,
            base_revision,
        } => {
            handle_apply_helm_upgrade(
                state,
                sender,
                namespace,
                release,
                values_yaml,
                chart_version,
                base_revision,
            )
            .await?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

/// Applications view (US1/US2) and the generic related-resources resolver (US3) — both
/// strictly read-only, synthesized from already-collected data (spec 004).
async fn handle_applications_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshApplications => {
            applications::refresh_applications(state, sender).await?
        }
        BackendCmd::SelectApplication {
            namespace,
            instance,
        } => {
            applications::select_application(state, sender, &namespace, &instance).await?;
        }
        BackendCmd::RefreshRelatedResources {
            section,
            kind,
            namespace,
            name,
        } => {
            let related = related::related_resources(state, &kind, namespace.as_deref(), &name)
                .await
                .unwrap_or_default();
            crate::backend::events::emit(
                state.cluster.seq.as_ref(),
                sender,
                crate::protocol::Route::namespaced(
                    section.clone(),
                    state.cluster.current_context.clone(),
                    namespace.clone(),
                ),
                crate::protocol::Correlation::None,
                crate::protocol::BackendEvent::ResourceRead(
                    crate::protocol::ResourceReadEvent::RelatedResourcesUpdated {
                        section,
                        kind,
                        namespace,
                        name,
                        related,
                    },
                ),
            );
        }
        BackendCmd::SelectFluxResource {
            kind,
            namespace,
            name,
        } => {
            handle_select_flux_resource(state, sender, kind, namespace, name).await?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

/// `PreviewHelmUpgrade` (006-helm-mutate T026): renders the diff via `helm upgrade --dry-run`
/// (backend/helm.rs) without mutating anything, and reports the result (or failure) as a
/// `SystemEvent` on the release's own namespaced route.
async fn handle_preview_helm_upgrade(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    release: String,
    values_yaml: String,
    chart_version: Option<String>,
) -> anyhow::Result<()> {
    let event = match helm::preview_helm_upgrade(
        state,
        &namespace,
        &release,
        &values_yaml,
        chart_version.as_deref(),
    )
    .await
    {
        Ok(preview) => crate::protocol::SystemEvent::HelmMutationPreviewReady(preview),
        Err(failed) => crate::protocol::SystemEvent::HelmMutationFailed(failed),
    };
    crate::backend::events::emit(
        state.cluster.seq.as_ref(),
        sender,
        crate::protocol::Route::namespaced(
            crate::protocol::section_keys::helm_releases(),
            state.cluster.current_context.clone(),
            Some(namespace),
        ),
        crate::protocol::Correlation::None,
        crate::protocol::BackendEvent::System(event),
    );
    Ok(())
}

/// `ApplyHelmUpgrade` (006-helm-mutate T026): re-checks `base_revision`, applies the real
/// `helm upgrade`, reports the result as a `SystemEvent`, and refreshes the release's details so
/// the Inspector reflects the new revision without a separate manual refresh.
async fn handle_apply_helm_upgrade(
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    release: String,
    values_yaml: String,
    chart_version: Option<String>,
    base_revision: i64,
) -> anyhow::Result<()> {
    let event = match helm::apply_helm_upgrade(
        state,
        &namespace,
        &release,
        &values_yaml,
        chart_version.as_deref(),
        base_revision,
    )
    .await
    {
        Ok(applied) => crate::protocol::SystemEvent::HelmMutationApplied(applied),
        Err(failed) => crate::protocol::SystemEvent::HelmMutationFailed(failed),
    };
    crate::backend::events::emit(
        state.cluster.seq.as_ref(),
        sender,
        crate::protocol::Route::namespaced(
            crate::protocol::section_keys::helm_releases(),
            state.cluster.current_context.clone(),
            Some(namespace.clone()),
        ),
        crate::protocol::Correlation::None,
        crate::protocol::BackendEvent::System(event),
    );
    helm::get_release_details(state, sender, &namespace, &release).await?;
    Ok(())
}

/// `SelectFluxResource`: re-runs the existing Flux discovery/summarize functions (no new poll
/// loop, no new Kubernetes watch — research.md R3) and filters for the requested object,
/// emitting `FluxResourceNotFound` rather than silently doing nothing when it's gone (FR-013).
async fn handle_select_flux_resource(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    kind: String,
    namespace: String,
    name: String,
) -> anyhow::Result<()> {
    let client = state.cluster.client_or_err()?;
    let (kustomizations, helmreleases) =
        flux::list_kustomizations_and_helmreleases(&client).await?;
    let candidates = if kind == "Kustomization" {
        &kustomizations
    } else {
        &helmreleases
    };
    let found = find_flux_candidate(candidates, &namespace, &name);

    let event = match found {
        Some(obj) => crate::protocol::ResourceReadEvent::FluxResourceDetailsUpdated(
            flux::summarize(obj, &kind, false),
        ),
        None => crate::protocol::ResourceReadEvent::FluxResourceNotFound {
            kind: kind.clone(),
            namespace: namespace.clone(),
            name: name.clone(),
        },
    };

    crate::backend::events::emit(
        state.cluster.seq.as_ref(),
        sender,
        crate::protocol::Route::namespaced(
            crate::protocol::section_keys::flux_resource(),
            state.cluster.current_context.clone(),
            Some(namespace),
        ),
        crate::protocol::Correlation::None,
        crate::protocol::BackendEvent::ResourceRead(event),
    );
    Ok(())
}

/// Emits a `HelmMutationPreviewReady` on success or `HelmMutationFailed` on failure (006 US1),
/// routed to the Helm Releases section the same way `SelectHelmRelease` is.
fn emit_helm_mutation_preview(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    result: Result<crate::protocol::HelmMutationPreviewReady, crate::protocol::HelmMutationFailed>,
) {
    let event = match result {
        Ok(preview) => crate::protocol::SystemEvent::HelmMutationPreviewReady(preview),
        Err(failed) => crate::protocol::SystemEvent::HelmMutationFailed(failed),
    };
    crate::backend::events::emit(
        state.cluster.seq.as_ref(),
        sender,
        crate::protocol::Route::namespaced(
            crate::protocol::section_keys::helm_releases(),
            state.cluster.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        crate::protocol::Correlation::None,
        crate::protocol::BackendEvent::System(event),
    );
}

/// Emits a `HelmMutationApplied` on success or `HelmMutationFailed` on failure (006 US1).
fn emit_helm_mutation_applied(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: &str,
    result: Result<crate::protocol::HelmMutationApplied, crate::protocol::HelmMutationFailed>,
) {
    let event = match result {
        Ok(applied) => crate::protocol::SystemEvent::HelmMutationApplied(applied),
        Err(failed) => crate::protocol::SystemEvent::HelmMutationFailed(failed),
    };
    crate::backend::events::emit(
        state.cluster.seq.as_ref(),
        sender,
        crate::protocol::Route::namespaced(
            crate::protocol::section_keys::helm_releases(),
            state.cluster.current_context.clone(),
            Some(namespace.to_owned()),
        ),
        crate::protocol::Correlation::None,
        crate::protocol::BackendEvent::System(event),
    );
}

/// `UninstallHelmRelease` (006 T033): shells out to `helm uninstall` via
/// `helm::uninstall_helm_release`, then emits `HelmMutationApplied`/`HelmMutationFailed` on the
/// release's own `helm/releases` route so the frontend's already-open Inspector (if any) updates
/// without a separate refresh.
async fn handle_uninstall_helm_release(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    namespace: String,
    release: String,
    base_revision: i64,
) -> anyhow::Result<()> {
    let route = crate::protocol::Route::namespaced(
        crate::protocol::section_keys::helm_releases(),
        state.cluster.current_context.clone(),
        Some(namespace.clone()),
    );

    let client = match state.cluster.client_or_err() {
        Ok(client) => client,
        Err(e) => {
            emit_helm_mutation_failed(state, sender, route, namespace, release, format!("{e}"));
            return Ok(());
        }
    };

    match helm::uninstall_helm_release(&client, &namespace, &release, base_revision).await {
        Ok(applied) => {
            crate::backend::events::emit(
                state.cluster.seq.as_ref(),
                sender,
                route,
                crate::protocol::Correlation::None,
                crate::protocol::BackendEvent::System(
                    crate::protocol::SystemEvent::HelmMutationApplied(applied),
                ),
            );
        }
        Err(failed) => {
            crate::backend::events::emit(
                state.cluster.seq.as_ref(),
                sender,
                route,
                crate::protocol::Correlation::None,
                crate::protocol::BackendEvent::System(
                    crate::protocol::SystemEvent::HelmMutationFailed(failed),
                ),
            );
        }
    }
    Ok(())
}

fn emit_helm_mutation_failed(
    state: &BackendState,
    sender: &Sender<BackendEventEnvelope>,
    route: crate::protocol::Route,
    namespace: String,
    release: String,
    message: String,
) {
    crate::backend::events::emit(
        state.cluster.seq.as_ref(),
        sender,
        route,
        crate::protocol::Correlation::None,
        crate::protocol::BackendEvent::System(crate::protocol::SystemEvent::HelmMutationFailed(
            crate::protocol::HelmMutationFailed {
                namespace,
                release,
                action: crate::protocol::HelmMutationAction::Uninstall,
                stage: crate::protocol::HelmMutationStage::Apply,
                message,
            },
        )),
    );
}

fn find_flux_candidate<'a>(
    candidates: &'a [kube::core::DynamicObject],
    namespace: &str,
    name: &str,
) -> Option<&'a kube::core::DynamicObject> {
    candidates.iter().find(|o| {
        o.metadata.namespace.as_deref() == Some(namespace)
            && o.metadata.name.as_deref() == Some(name)
    })
}

/// Generic, read-only Custom Resource Definition browsing (US3): discovers every installed
/// CRD and lists/reads its instances through `crds.rs` rather than one hardcoded module per
/// CRD kind. `SelectCrdKind` is intentionally a no-op here: the frontend tracks the selected
/// kind itself and follows up with `RefreshCrdInstances`, so no backend-held "current kind"
/// state is needed.
async fn handle_crd_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshCrds => crds::refresh_crds(state, sender).await?,
        BackendCmd::SelectCrdKind { .. } => {}
        BackendCmd::RefreshCrdInstances {
            group,
            kind,
            version,
            namespace,
        } => {
            crds::refresh_crd_instances(
                state,
                sender,
                &group,
                &version,
                &kind,
                namespace.as_deref(),
            )
            .await?;
        }
        BackendCmd::SelectCrdInstance {
            group,
            kind,
            version,
            namespace,
            name,
        } => {
            crds::select_crd_instance(
                state,
                sender,
                &group,
                &version,
                &kind,
                namespace.as_deref(),
                &name,
            )
            .await?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn handle_network_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshServices => services::refresh_services(state, sender).await?,
        BackendCmd::SelectService { namespace, name } => {
            services::fetch_service_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteService { namespace, name } => {
            services::delete_service(state, sender, &namespace, &name).await?;
        }
        BackendCmd::RefreshEndpoints => endpoints::refresh_endpoints(state, sender).await?,
        BackendCmd::SelectEndpoint { namespace, name } => {
            endpoints::fetch_endpoint_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::RefreshIngresses => ingresses::refresh_ingresses(state, sender).await?,
        BackendCmd::SelectIngress { namespace, name } => {
            ingresses::fetch_ingress_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteIngress { namespace, name } => {
            ingresses::delete_ingress(state, sender, &namespace, &name).await?;
        }
        BackendCmd::RefreshIngressClasses => {
            ingressclasses::refresh_ingress_classes(state, sender).await?;
        }
        BackendCmd::SelectIngressClass { name } => {
            ingressclasses::fetch_ingress_class_details(state, sender, &name).await?;
        }
        BackendCmd::RefreshNetworkPolicies => {
            networkpolicies::refresh_network_policies(state, sender).await?;
        }
        BackendCmd::SelectNetworkPolicy { namespace, name } => {
            networkpolicies::fetch_network_policy_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteNetworkPolicy { namespace, name } => {
            networkpolicies::delete_network_policy(state, sender, &namespace, &name).await?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn handle_storage_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshPersistentVolumes => {
            persistentvolumes::refresh_persistentvolumes(state, sender).await?;
        }
        BackendCmd::SelectPersistentVolume { name } => {
            persistentvolumes::fetch_persistentvolume_details(state, sender, &name).await?;
        }
        BackendCmd::DeletePersistentVolume { name } => {
            persistentvolumes::delete_persistentvolume(state, sender, &name).await?;
            persistentvolumes::refresh_persistentvolumes(state, sender).await?;
        }
        BackendCmd::RefreshPersistentVolumeClaims => {
            persistentvolumeclaims::refresh_persistentvolumeclaims(state, sender).await?;
        }
        BackendCmd::SelectPersistentVolumeClaim { namespace, name } => {
            persistentvolumeclaims::fetch_persistentvolumeclaim_details(
                state, sender, &namespace, &name,
            )
            .await?;
        }
        BackendCmd::DeletePersistentVolumeClaim { namespace, name } => {
            persistentvolumeclaims::delete_persistentvolumeclaim(state, sender, &namespace, &name)
                .await?;
            persistentvolumeclaims::refresh_persistentvolumeclaims(state, sender).await?;
        }
        BackendCmd::RefreshStorageClasses => {
            storageclasses::refresh_storageclasses(state, sender).await?;
        }
        BackendCmd::SelectStorageClass { name } => {
            storageclasses::fetch_storageclass_details(state, sender, &name).await?;
        }
        BackendCmd::DeleteStorageClass { name } => {
            storageclasses::delete_storageclass(state, sender, &name).await?;
            storageclasses::refresh_storageclasses(state, sender).await?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn handle_config_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshConfigMaps => configmaps::refresh_configmaps(state, sender).await?,
        BackendCmd::SelectConfigMap { namespace, name } => {
            configmaps::fetch_configmap_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteConfigMap { namespace, name } => {
            configmaps::delete_configmap(state, sender, &namespace, &name).await?;
            configmaps::refresh_configmaps(state, sender).await?;
        }
        BackendCmd::RefreshSecrets => secrets::refresh_secrets(state, sender).await?,
        BackendCmd::SelectSecret { namespace, name } => {
            secrets::fetch_secret_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteSecret { namespace, name } => {
            secrets::delete_secret(state, sender, &namespace, &name).await?;
            secrets::refresh_secrets(state, sender).await?;
        }
        BackendCmd::RefreshResourceQuotas => {
            resourcequotas::refresh_resourcequotas(state, sender).await?;
        }
        BackendCmd::SelectResourceQuota { namespace, name } => {
            resourcequotas::fetch_resourcequota_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteResourceQuota { namespace, name } => {
            resourcequotas::delete_resourcequota(state, sender, &namespace, &name).await?;
            resourcequotas::refresh_resourcequotas(state, sender).await?;
        }
        BackendCmd::RefreshLimitRanges => limitranges::refresh_limitranges(state, sender).await?,
        BackendCmd::SelectLimitRange { namespace, name } => {
            limitranges::fetch_limitrange_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteLimitRange { namespace, name } => {
            limitranges::delete_limitrange(state, sender, &namespace, &name).await?;
            limitranges::refresh_limitranges(state, sender).await?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn handle_policy_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshHorizontalPodAutoscalers => {
            horizontalpodautoscalers::refresh_horizontalpodautoscalers(state, sender).await?;
        }
        BackendCmd::SelectHorizontalPodAutoscaler { namespace, name } => {
            horizontalpodautoscalers::fetch_horizontalpodautoscaler_details(
                state, sender, &namespace, &name,
            )
            .await?;
        }
        BackendCmd::DeleteHorizontalPodAutoscaler { namespace, name } => {
            horizontalpodautoscalers::delete_horizontalpodautoscaler(
                state, sender, &namespace, &name,
            )
            .await?;
            horizontalpodautoscalers::refresh_horizontalpodautoscalers(state, sender).await?;
        }
        BackendCmd::RefreshPodDisruptionBudgets => {
            poddisruptionbudgets::refresh_poddisruptionbudgets(state, sender).await?;
        }
        BackendCmd::SelectPodDisruptionBudget { namespace, name } => {
            poddisruptionbudgets::fetch_poddisruptionbudget_details(
                state, sender, &namespace, &name,
            )
            .await?;
        }
        BackendCmd::DeletePodDisruptionBudget { namespace, name } => {
            poddisruptionbudgets::delete_poddisruptionbudget(state, sender, &namespace, &name)
                .await?;
            poddisruptionbudgets::refresh_poddisruptionbudgets(state, sender).await?;
        }
        BackendCmd::RefreshPriorityClasses => {
            priorityclasses::refresh_priorityclasses(state, sender).await?;
        }
        BackendCmd::SelectPriorityClass { name } => {
            priorityclasses::fetch_priorityclass_details(state, sender, &name).await?;
        }
        BackendCmd::DeletePriorityClass { name } => {
            priorityclasses::delete_priorityclass(state, sender, &name).await?;
            priorityclasses::refresh_priorityclasses(state, sender).await?;
        }
        BackendCmd::RefreshRuntimeClasses => {
            runtimeclasses::refresh_runtimeclasses(state, sender).await?;
        }
        BackendCmd::SelectRuntimeClass { name } => {
            runtimeclasses::fetch_runtimeclass_details(state, sender, &name).await?;
        }
        BackendCmd::DeleteRuntimeClass { name } => {
            runtimeclasses::delete_runtimeclass(state, sender, &name).await?;
            runtimeclasses::refresh_runtimeclasses(state, sender).await?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn handle_admission_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshLeases => leases::refresh_leases(state, sender).await?,
        BackendCmd::SelectLease { namespace, name } => {
            leases::fetch_lease_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteLease { namespace, name } => {
            leases::delete_lease(state, sender, &namespace, &name).await?;
            leases::refresh_leases(state, sender).await?;
        }
        BackendCmd::RefreshMutatingWebhookConfigurations => {
            mutatingwebhooks::refresh_mutatingwebhooks(state, sender).await?;
        }
        BackendCmd::SelectMutatingWebhookConfiguration { name } => {
            mutatingwebhooks::fetch_mutatingwebhook_details(state, sender, &name).await?;
        }
        BackendCmd::DeleteMutatingWebhookConfiguration { name } => {
            mutatingwebhooks::delete_mutatingwebhook(state, sender, &name).await?;
            mutatingwebhooks::refresh_mutatingwebhooks(state, sender).await?;
        }
        BackendCmd::RefreshValidatingWebhookConfigurations => {
            validatingwebhooks::refresh_validatingwebhooks(state, sender).await?;
        }
        BackendCmd::SelectValidatingWebhookConfiguration { name } => {
            validatingwebhooks::fetch_validatingwebhook_details(state, sender, &name).await?;
        }
        BackendCmd::DeleteValidatingWebhookConfiguration { name } => {
            validatingwebhooks::delete_validatingwebhook(state, sender, &name).await?;
            validatingwebhooks::refresh_validatingwebhooks(state, sender).await?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

async fn handle_access_cmd(
    cmd: BackendCmd,
    state: &mut BackendState,
    sender: &Sender<BackendEventEnvelope>,
) -> anyhow::Result<Option<BackendCmd>> {
    match cmd {
        BackendCmd::RefreshRoles => roles::refresh_roles(state, sender).await?,
        BackendCmd::SelectRole { namespace, name } => {
            roles::fetch_role_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteRole { namespace, name } => {
            roles::delete_role(state, sender, &namespace, &name).await?;
            roles::refresh_roles(state, sender).await?;
        }
        BackendCmd::RefreshRoleBindings => {
            rolebindings::refresh_role_bindings(state, sender).await?
        }
        BackendCmd::SelectRoleBinding { namespace, name } => {
            rolebindings::fetch_role_binding_details(state, sender, &namespace, &name).await?;
        }
        BackendCmd::DeleteRoleBinding { namespace, name } => {
            rolebindings::delete_role_binding(state, sender, &namespace, &name).await?;
            rolebindings::refresh_role_bindings(state, sender).await?;
        }
        BackendCmd::RefreshClusterRoles => {
            clusterroles::refresh_cluster_roles(state, sender).await?
        }
        BackendCmd::SelectClusterRole { name } => {
            clusterroles::fetch_cluster_role_details(state, sender, &name).await?;
        }
        BackendCmd::DeleteClusterRole { name } => {
            clusterroles::delete_cluster_role(state, sender, &name).await?;
            clusterroles::refresh_cluster_roles(state, sender).await?;
        }
        BackendCmd::RefreshClusterRoleBindings => {
            clusterrolebindings::refresh_cluster_role_bindings(state, sender).await?;
        }
        BackendCmd::SelectClusterRoleBinding { name } => {
            clusterrolebindings::fetch_cluster_role_binding_details(state, sender, &name).await?;
        }
        BackendCmd::DeleteClusterRoleBinding { name } => {
            clusterrolebindings::delete_cluster_role_binding(state, sender, &name).await?;
            clusterrolebindings::refresh_cluster_role_bindings(state, sender).await?;
        }
        BackendCmd::RefreshRbacSelfAccess { checks } => {
            rbac_access::handle_refresh_rbac_self_access(state, sender, checks).await?;
        }
        BackendCmd::CheckResourceMutationAccess {
            api_version,
            kind,
            namespace,
            verb,
        } => {
            rbac_access::handle_check_resource_mutation_access(
                state,
                sender,
                api_version,
                kind,
                namespace,
                verb,
            )
            .await?;
        }
        BackendCmd::ResolveNamespaceAccess {
            subject_kind,
            subject_name,
            subject_namespace,
        } => {
            rbac_access::handle_resolve_namespace_access(
                state,
                sender,
                subject_kind,
                subject_name,
                subject_namespace,
            )
            .await?;
        }
        BackendCmd::RefreshKubeadmDetection => {
            rbac_access::handle_refresh_kubeadm_detection(state, sender).await?;
        }
        other => return Ok(Some(other)),
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::core::{ApiResource, DynamicObject};

    fn dynamic_object(namespace: &str, name: &str) -> DynamicObject {
        let api_resource = ApiResource {
            group: "helm.toolkit.fluxcd.io".to_owned(),
            version: "v2".to_owned(),
            kind: "HelmRelease".to_owned(),
            api_version: "helm.toolkit.fluxcd.io/v2".to_owned(),
            plural: "helmreleases".to_owned(),
        };
        DynamicObject::new(name, &api_resource).within(namespace)
    }

    #[test]
    fn find_flux_candidate_locates_matching_namespace_and_name() {
        let candidates = vec![dynamic_object("flux-system", "redis")];
        let found = find_flux_candidate(&candidates, "flux-system", "redis");
        assert!(found.is_some());
    }

    #[test]
    fn find_flux_candidate_none_for_deleted_origin() {
        let candidates = vec![dynamic_object("flux-system", "redis")];
        let found = find_flux_candidate(&candidates, "flux-system", "gone");
        assert!(found.is_none());
    }

    #[test]
    fn find_flux_candidate_none_for_empty_list() {
        let candidates: Vec<DynamicObject> = Vec::new();
        assert!(find_flux_candidate(&candidates, "flux-system", "redis").is_none());
    }
}
