import { useEffect, useMemo, useState } from "react";
import { Button, NativeSelect, TextInput } from "@mantine/core";
import { desktopApi } from "../../lib/desktop-api";
import type { DesktopState } from "../../models/topology";
import type { UnregisterObservation, WorkspaceProject } from "../../models/workspace";
import { useProduct } from "../../i18n/product";
import { displayProjectPath, projectName, runnerInventoryComplete, useWorkspace } from "../workspace/WorkspaceContext";
import { WorkspaceDialog } from "../workspace/WorkspaceDialog";
import { ProjectRows } from "./ProjectRows";
import { RunnerDevices } from "./RunnerDevices";
import { projectRunnerId } from "./project-device";
import { AddLocalProject } from "./AddLocalProject";
import { AddDevice } from "./AddDevice";

export function ProjectsPanel({ onComputerSettings, onState }: { onComputerSettings?: () => void; onState?: (state: DesktopState) => void }) {
  const p = useProduct();
  const workspace = useWorkspace();
  const [query, setQuery] = useState("");
  const [deviceSelection, setDeviceSelection] = useState<{ context: string; id: string } | null>(null);
  const authorizationLost = workspace.error && ["authenticationRequired", "permissionDenied"].includes(workspace.errorReason);
  const selectedDevice = !authorizationLost && deviceSelection?.context === workspace.contextKey ? deviceSelection.id : "";
  const observedDeviceIds = useMemo(() => Array.from(new Set([
    ...workspace.runners.map(runner => runner.client_id),
    ...workspace.projects.map(projectRunnerId).filter((id): id is string => Boolean(id)),
  ])).sort(), [workspace.runners, workspace.projects]);
  const inventoryComplete = !workspace.loading && !workspace.fleetStale && runnerInventoryComplete(workspace.runner);
  // Derive the displayed filter from the same inventory as the results, before
  // the cleanup effect. A removed option must never make the select show All
  // while the rows still use the previous device.
  const device = inventoryComplete && !observedDeviceIds.includes(selectedDevice) ? "" : selectedDevice;
  // An incomplete refresh cannot revoke a prior observation. Keep the selected
  // option alongside the filter so the select never silently displays All.
  const deviceIds = device && !inventoryComplete && !observedDeviceIds.includes(device)
    ? [...observedDeviceIds, device].sort() : observedDeviceIds;
  useEffect(() => {
    if (!deviceSelection) return;
    if (deviceSelection.context !== workspace.contextKey
      || authorizationLost
      || (inventoryComplete && !observedDeviceIds.includes(deviceSelection.id))) {
      setDeviceSelection(null);
    }
  }, [deviceSelection, workspace.contextKey, authorizationLost, inventoryComplete, observedDeviceIds]);
  const selectDevice = (id: string) => setDeviceSelection(id ? { context: workspace.contextKey, id } : null);
  const fromDeviceCard = (id: string) => {
    selectDevice(id);
    const filter = document.getElementById("projects-device-filter");
    filter?.focus();
    filter?.scrollIntoView?.({ block: "nearest" });
  };
  const [removal, setRemoval] = useState<UnregisterObservation | null>(null);
  const [removing, setRemoving] = useState(false);
  const [removeError, setRemoveError] = useState(false);
  const [selected, setSelected] = useState<string[]>([]);
  const [batchConfirm, setBatchConfirm] = useState(false);
  const [batchProgress, setBatchProgress] = useState("");
  const [batchIssues, setBatchIssues] = useState<string[]>([]);
  const canManageProjects = Boolean(onState && workspace.state.workspace_runner);

  const prepareRemoval = async (project: WorkspaceProject) => {
    if (!canManageProjects || removing || workspace.state.current_operation || !project.id) return;
    setRemoving(true);
    setRemoveError(false);
    try {
      setRemoval(await desktopApi.prepareProjectUnregister(project.id));
    } catch {
      setRemoveError(true);
    } finally {
      setRemoving(false);
    }
  };

  const unregister = async () => {
    if (!removal || !onState || removing || workspace.state.current_operation) return;
    setRemoving(true);
    setRemoveError(false);
    try {
      onState(await desktopApi.unregisterProject(removal));
      workspace.removeProject(removal.project);
    } catch {
      setRemoveError(true);
    } finally {
      setRemoval(null);
      setRemoving(false);
      workspace.refresh();
    }
  };

  const unregisterSelected = async () => {
    if (!onState || removing || workspace.state.current_operation) return;
    const ids = selected.filter(id => workspace.projects.some(project => project.id === id && project.connected && projectRunnerId(project) === workspace.state.workspace_runner?.client_id));
    if (!ids.length) return;
    setRemoving(true);
    setRemoveError(false);
    setBatchIssues([]);
    setBatchConfirm(false);
    try {
      let completed = 0;
      for (let offset = 0; offset < ids.length; offset += 16) {
        const chunk = ids.slice(offset, offset + 16);
        const observations = await desktopApi.prepareProjectsUnregister(chunk);
        setBatchProgress(`${completed}/${ids.length}`);
        if (workspace.state.persistent_environment) {
          for (const observation of observations) {
            try {
              onState(await desktopApi.unregisterProject(observation));
              workspace.removeProject(observation.project);
              setSelected(current => current.filter(id => id !== observation.project));
              completed++;
              setBatchProgress(`${completed}/${ids.length}`);
            } catch {
              setRemoveError(true);
              setBatchIssues([observation.project]);
              return;
            }
          }
        } else {
          const result = await desktopApi.unregisterProjects(observations);
          onState(result.state);
          const succeeded = result.items.filter(item => item.success).map(item => item.project);
          for (const id of succeeded) workspace.removeProject(id);
          setSelected(current => current.filter(id => !succeeded.includes(id)));
          completed += succeeded.length;
          setBatchProgress(`${completed}/${ids.length}`);
          const issues = result.items.filter(item => !item.success).map(item => `${item.project}: ${item.status}`);
          if (issues.length || result.items.length !== chunk.length) {
            setBatchIssues(issues.length ? issues : chunk);
            setRemoveError(true);
            return;
          }
        }
      }
    } catch {
      setRemoveError(true);
    } finally {
      setBatchProgress("");
      setRemoving(false);
      workspace.refresh();
    }
  };

  const rows = workspace.projects.filter(project =>
    (!device || projectRunnerId(project) === device) &&
    `${projectName(project)} ${displayProjectPath(project.path)} ${projectRunnerId(project) || ""}`
      .toLocaleLowerCase()
      .includes(query.trim().toLocaleLowerCase()),
  );
  return <section className="page-section workspace-page" aria-labelledby="projects-title" data-webcodex-page="projects">
    <header className="page-heading-row"><h1 id="projects-title">{p("projects")} <span className="heading-count">{workspace.projects.length}</span></h1><button type="button" className="secondary-button" onClick={workspace.refresh} disabled={workspace.busy}>{p("refresh")}</button></header>
    {onState && <AddLocalProject state={workspace.state} onState={onState} onAdded={workspace.refresh} />}
    {workspace.runner && <p className="workspace-notice">{p("projectAccessScope")}</p>}
    <div className="workspace-project-filters">
      <div className="workspace-search"><TextInput id="projects-search" label={p("search")} type="search" value={query} onChange={event => setQuery(event.currentTarget.value)} /></div>
      <div className="workspace-search"><NativeSelect size="md" classNames={{ input: "ui-mantine-input", label: "ui-mantine-label" }} id="projects-device-filter" label={p("executionDevice")} value={device} onChange={event => selectDevice(event.currentTarget.value)} data={[
        { value: "", label: p("allDevices") },
        ...deviceIds.map(id => ({ value: id, label: id === workspace.state.workspace_runner?.client_id ? `${p("thisComputer")} · ${id}` : id })),
      ]} /></div>
    </div>
    <div className="project-filter-summary"><span role="status">{p("projectFilterCount").replace("{shown}", String(rows.length)).replace("{total}", String(workspace.projects.length))}</span>
      {(device || query) && <button type="button" className="text-button" onClick={() => { selectDevice(""); setQuery(""); }}>{p("clearProjectFilters")}</button>}
    </div>
    {workspace.error && <div className="workspace-notice" role="alert">{p(workspace.errorReason)}</div>}
    {canManageProjects && <div className="shell-actions">
      <Button variant="default" disabled={removing || !rows.some(project => project.id && project.connected && projectRunnerId(project) === workspace.state.workspace_runner?.client_id)} onClick={() => {
        const visible = rows.filter(project => project.id && project.connected && projectRunnerId(project) === workspace.state.workspace_runner?.client_id).map(project => project.id);
        setSelected(current => visible.every(id => current.includes(id)) ? current.filter(id => !visible.includes(id)) : Array.from(new Set([...current, ...visible])));
      }}>{p("selectVisibleProjects")}</Button>
      <Button color="red" disabled={removing || !selected.some(id => workspace.projects.some(project => project.id === id))} onClick={() => setBatchConfirm(true)}>{p("removeSelectedProjects").replace("{count}", String(selected.length))}</Button>
      {batchProgress && <span role="status">{batchProgress}</span>}
    </div>}
    <ProjectRows projects={rows} onUnregister={canManageProjects ? prepareRemoval : undefined} onSelect={canManageProjects ? (id, checked) => setSelected(current => checked ? Array.from(new Set([...current, id])) : current.filter(value => value !== id)) : undefined} selected={selected} busy={removing || Boolean(workspace.state.current_operation)} />
    {removeError && <p role="alert" className="workspace-notice">{p("unregisterError")}</p>}
    {batchIssues.length > 0 && <ul aria-label={p("failedProjects")} className="workspace-notice">{batchIssues.map(issue => <li key={issue}>{issue}</li>)}</ul>}
    {batchConfirm && <WorkspaceDialog title={p("removeSelectedProjects").replace("{count}", String(selected.length))} onClose={() => setBatchConfirm(false)} busy={removing}>
      <p>{p("unregisterDescription")}</p>
      <p>{selected.map(id => projectName(workspace.projects.find(project => project.id === id) || { id, connected: false })).join(", ")}</p>
      <div className="shell-actions">
        <Button variant="default" onClick={() => setBatchConfirm(false)}>{p("cancel")}</Button>
        <Button color="red" onClick={() => void unregisterSelected()}>{p("removeSelectedProjects").replace("{count}", String(selected.length))}</Button>
      </div>
    </WorkspaceDialog>}
    {removal && <WorkspaceDialog title={p("unregisterProject")} onClose={() => setRemoval(null)} busy={removing}>
      <p>{p("unregisterDescription")}</p>
      <code>{displayProjectPath(removal.path)}</code>
      <div className="shell-actions">
        <Button variant="default" onClick={() => setRemoval(null)} disabled={removing}>{p("cancel")}</Button>
        <Button color="red" onClick={() => void unregister()} disabled={removing || Boolean(workspace.state.current_operation)}>{p("unregisterProject")}</Button>
      </div>
    </WorkspaceDialog>}
    {!rows.length && !workspace.error && !workspace.loading && <p className="workspace-empty">{p(query || device ? "noMatches" : "noProjects")}</p>}    {workspace.runner?.projects_truncated && <p className="workspace-notice">{p("partial")}</p>}
    <AddDevice state={workspace.state} onViewDevices={workspace.refresh} />
    <RunnerDevices onComputerSettings={onComputerSettings} selectedDevice={device} onSelectDevice={fromDeviceCard} />
  </section>;
}
