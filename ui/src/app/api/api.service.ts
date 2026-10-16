import { Injectable, inject } from '@angular/core';
import { HttpClient, HttpErrorResponse, HttpParams } from '@angular/common/http';
import { Observable, catchError, throwError } from 'rxjs';
import type {
  ApiError, AppState, BrowseResult, ConfigOverview, DeployPreflight, DeployRequest, EditOp, Generation,
  JobDetail, JobInfo, LocationInfo, LocationSuggestion, MachineInfo, NewFileRequest, NixFileView, RenameResult,
  OptionDoc, OptionsStatus, PackageSearchResult, Recipe, RenderResult, ConfigChange,
  FlakeInfo, FlakeOp, ConvertToFlakeRequest, ChannelsInfo, ChannelActionRequest, PackageSet, DiskInfo, OrganizeResult,
  SecretsStatus, SecretEntry, DeployTarget, RemoteCheck, HardwareSync, HardwarePullResult,
  GitInfo, GitBranch, GitSuggestion, GitNetResult,
  JobLogFile,
} from './generated';

export class ApiFailure extends Error {
  constructor(public readonly status: number, public readonly code: string, message: string) {
    super(message);
  }
}

@Injectable({ providedIn: 'root' })
export class ApiService {
  private http = inject(HttpClient);
  private base = '/api';

  private wrap<T>(obs: Observable<T>): Observable<T> {
    return obs.pipe(
      catchError((err: HttpErrorResponse) => {
        const body = err.error as Partial<ApiError> | undefined;
        const message = body?.error ?? err.message ?? 'request failed';
        return throwError(() => new ApiFailure(err.status, body?.code ?? 'http', message));
      }),
    );
  }

  state() { return this.wrap(this.http.get<AppState>(`${this.base}/state`)); }
  forgetLocation(path: string) { return this.wrap(this.http.post<AppState>(`${this.base}/state/forget`, { path })); }

  browse(path?: string) {
    const params = path ? new HttpParams().set('path', path) : undefined;
    return this.wrap(this.http.get<BrowseResult>(`${this.base}/fs/browse`, { params }));
  }
  suggestions() { return this.wrap(this.http.get<LocationSuggestion[]>(`${this.base}/fs/suggestions`)); }
  pullHardware(target: string, path: string) { return this.wrap(this.http.post<HardwarePullResult>(`${this.base}/remote/pull-hardware`, { target, path })); }
  jobHistory() { return this.wrap(this.http.get<JobLogFile[]>(`${this.base}/jobs/history`)); }
  jobHistoryDelete(name: string) { return this.wrap(this.http.delete<{ deleted: number }>(`${this.base}/jobs/history/${encodeURIComponent(name)}`)); }
  jobHistoryClear() { return this.wrap(this.http.delete<{ deleted: number; bytes: number }>(`${this.base}/jobs/history`)); }
  jobHistoryRead(name: string) { return this.wrap(this.http.get<string>(`${this.base}/jobs/history/${encodeURIComponent(name)}`)); }
  stopJob(id: string) { return this.wrap(this.http.post<JobInfo>(`${this.base}/jobs/${encodeURIComponent(id)}/stop`, {})); }
  // ── git ──
  gitInfo(path: string) { return this.wrap(this.http.get<GitInfo>(`${this.base}/git/info`, { params: { path } })); }
  gitBranches(path: string) { return this.wrap(this.http.get<GitBranch[]>(`${this.base}/git/branches`, { params: { path } })); }
  gitDiff(path: string, file: string) { return this.wrap(this.http.get<string>(`${this.base}/git/diff`, { params: { path, file } })); }
  gitSuggest(path: string) { return this.wrap(this.http.get<GitSuggestion>(`${this.base}/git/suggest`, { params: { path } })); }
  gitSwitch(path: string, name: string) { return this.wrap(this.http.post<GitInfo>(`${this.base}/git/switch`, { path, name })); }
  gitCreateBranch(path: string, name: string) { return this.wrap(this.http.post<GitInfo>(`${this.base}/git/branch`, { path, name })); }
  gitCommit(path: string, message: string, paths: string[] | null, push: boolean, password: string | null) { return this.wrap(this.http.post<GitNetResult>(`${this.base}/git/commit`, { path, message, paths, push, password })); }
  gitPush(path: string, password: string | null) { return this.wrap(this.http.post<GitNetResult>(`${this.base}/git/push`, { path, password })); }
  gitFetch(path: string, password: string | null) { return this.wrap(this.http.post<GitNetResult>(`${this.base}/git/fetch`, { path, password })); }
  gitPull(path: string, password: string | null) { return this.wrap(this.http.post<GitNetResult>(`${this.base}/git/pull`, { path, password })); }
  gitIdentity(path: string, name: string, email: string) { return this.wrap(this.http.post<GitInfo>(`${this.base}/git/identity`, { path, name, email })); }
  hardwareSync(path: string) { return this.wrap(this.http.post<HardwareSync>(`${this.base}/location/hardware-sync`, { path })); }
  openLocation(path: string) { return this.wrap(this.http.post<LocationInfo>(`${this.base}/location/open`, { path })); }

  overview(path: string) {
    return this.wrap(this.http.get<ConfigOverview>(`${this.base}/config/overview`, { params: { path } }));
  }
  file(path: string, file: string) {
    return this.wrap(this.http.get<NixFileView>(`${this.base}/config/file`, { params: { path, file } }));
  }
  edit(path: string, file: string, version: string, op: EditOp) {
    return this.wrap(this.http.post<NixFileView>(`${this.base}/config/edit`, { path, file, version, op }));
  }
  organize(path: string, file: string, version: string, preview: boolean) {
    return this.wrap(this.http.post<OrganizeResult>(`${this.base}/config/organize`, { path, file, version, preview }));
  }
  newFile(req: NewFileRequest) { return this.wrap(this.http.post<NixFileView>(`${this.base}/config/new-file`, req)); }
  mkdir(path: string, target: string) { return this.wrap(this.http.post<{ ok: boolean }>(`${this.base}/config/fs/mkdir`, { path, target })); }
  rename(path: string, target: string, to: string, update_references = true) {
    return this.wrap(this.http.post<RenameResult>(`${this.base}/config/fs/rename`, { path, target, to, update_references }));
  }
  duplicate(path: string, target: string, to: string) { return this.wrap(this.http.post<{ ok: boolean }>(`${this.base}/config/fs/duplicate`, { path, target, to })); }
  delete(path: string, target: string, recursive = false) { return this.wrap(this.http.post<{ ok: boolean }>(`${this.base}/config/fs/delete`, { path, target, recursive })); }
  flakeHosts(path: string) { return this.wrap(this.http.get<string[]>(`${this.base}/config/flake-hosts`, { params: { path } })); }

  optionsStatus() { return this.wrap(this.http.get<OptionsStatus>(`${this.base}/options/status`)); }
  optionsBuild() { return this.wrap(this.http.post<OptionsStatus>(`${this.base}/options/build`, {})); }
  optionsSearch(q: string, limit = 40) { return this.wrap(this.http.get<OptionDoc[]>(`${this.base}/options/search`, { params: { q, limit } })); }
  optionGet(name: string) { return this.wrap(this.http.get<OptionDoc>(`${this.base}/options/get`, { params: { name } })); }
  packagesSearch(q: string, set = 'pkgs', path?: string) {
    const params: Record<string, string> = { q, set };
    if (path) params['path'] = path;
    return this.wrap(this.http.get<PackageSearchResult>(`${this.base}/packages/search`, { params }));
  }
  packageSets(path: string) { return this.wrap(this.http.get<PackageSet[]>(`${this.base}/config/package-sets`, { params: { path } })); }
  recipes() { return this.wrap(this.http.get<Recipe[]>(`${this.base}/wizard/recipes`)); }
  renderRecipe(recipe_id: string, values: Record<string, string>, package_set: string | null = null) {
    return this.wrap(this.http.post<RenderResult>(`${this.base}/wizard/render`, { recipe_id, values, package_set }));
  }
  applyChanges(path: string, file: string, version: string, changes: ConfigChange[]) {
    return this.wrap(this.http.post<NixFileView>(`${this.base}/wizard/apply`, { path, file, version, changes }));
  }
  flakeInfo(path: string) { return this.wrap(this.http.get<FlakeInfo>(`${this.base}/flake/info`, { params: { path } })); }
  flakeEdit(path: string, version: string, op: FlakeOp) { return this.wrap(this.http.post<FlakeInfo>(`${this.base}/flake/edit`, { path, version, op })); }
  flakeConvert(req: ConvertToFlakeRequest) { return this.wrap(this.http.post<FlakeInfo>(`${this.base}/flake/convert`, req)); }
  channels() { return this.wrap(this.http.get<ChannelsInfo>(`${this.base}/channels`)); }
  channelAction(req: ChannelActionRequest) { return this.wrap(this.http.post<JobInfo>(`${this.base}/channels/action`, req)); }
  secretsStatus(path: string) { return this.wrap(this.http.get<SecretsStatus>(`${this.base}/secrets/status`, { params: { path } })); }
  secretsCreate(path: string, password: string) { return this.wrap(this.http.post<SecretsStatus>(`${this.base}/secrets/create`, { path, password })); }
  secretsUnlock(path: string, password: string) { return this.wrap(this.http.post<SecretsStatus>(`${this.base}/secrets/unlock`, { path, password })); }
  secretsLock(path: string) { return this.wrap(this.http.post<SecretsStatus>(`${this.base}/secrets/lock`, { path })); }
  secretsGet(path: string, name: string) { return this.wrap(this.http.post<SecretEntry>(`${this.base}/secrets/get`, { path, name })); }
  secretsSet(path: string, entry: SecretEntry) { return this.wrap(this.http.post<SecretsStatus>(`${this.base}/secrets/set`, { path, entry })); }
  secretsRemove(path: string, name: string) { return this.wrap(this.http.post<SecretsStatus>(`${this.base}/secrets/remove`, { path, name })); }
  secretsChangePassword(path: string, password: string) { return this.wrap(this.http.post<SecretsStatus>(`${this.base}/secrets/change-password`, { path, password })); }
  disks() { return this.wrap(this.http.get<DiskInfo[]>(`${this.base}/machine/disks`)); }
  machine() { return this.wrap(this.http.get<MachineInfo>(`${this.base}/machine`)); }
  generations() { return this.wrap(this.http.get<Generation[]>(`${this.base}/generations`)); }

  deployPreflight(path: string) {
    return this.wrap(this.http.get<DeployPreflight>(`${this.base}/deploy/preflight`, { params: { path } }));
  }
  targets() { return this.wrap(this.http.get<DeployTarget[]>(`${this.base}/remote/targets`)); }
  saveTargets(targets: DeployTarget[]) { return this.wrap(this.http.put<DeployTarget[]>(`${this.base}/remote/targets`, targets)); }
  remoteTest(ssh_target: string) { return this.wrap(this.http.post<JobInfo>(`${this.base}/remote/test`, { ssh_target })); }
  remoteCheck(target: string) { return this.wrap(this.http.post<RemoteCheck>(`${this.base}/remote/check`, { target, password: '-' })); }
  setupKey(target: string, password: string) { return this.wrap(this.http.post<JobInfo>(`${this.base}/remote/setup-key`, { target, password })); }
  rootKey(password: string, builder_host: string | null) { return this.wrap(this.http.post<JobInfo>(`${this.base}/remote/root-key`, { password, builder_host })); }
  validate(path: string, host: string | null) { return this.wrap(this.http.post<JobInfo>(`${this.base}/validate`, { path, host })); }
  deploy(req: DeployRequest) { return this.wrap(this.http.post<JobInfo>(`${this.base}/deploy`, req)); }
  jobs() { return this.wrap(this.http.get<JobInfo[]>(`${this.base}/jobs`)); }
  job(id: string) { return this.wrap(this.http.get<JobDetail>(`${this.base}/jobs/${id}`)); }
}
