import { Routes } from '@angular/router';

export const routes: Routes = [
  { path: '', loadComponent: () => import('./pages/setup/setup.component').then(m => m.SetupComponent) },
  {
    path: 'w',
    loadComponent: () => import('./pages/workspace/workspace.component').then(m => m.WorkspaceComponent),
    children: [
      { path: '', pathMatch: 'full', redirectTo: 'overview' },
      { path: 'overview', loadComponent: () => import('./pages/overview/overview.component').then(m => m.OverviewComponent) },
      { path: 'file', loadComponent: () => import('./pages/file-view/file-view.component').then(m => m.FileViewComponent) },
      { path: 'flake', loadComponent: () => import('./pages/flake/flake.component').then(m => m.FlakeComponent) },
      { path: 'channels', loadComponent: () => import('./pages/channels/channels.component').then(m => m.ChannelsComponent) },
      { path: 'secrets', loadComponent: () => import('./pages/secrets/secrets.component').then(m => m.SecretsComponent) },
      { path: 'remote', loadComponent: () => import('./pages/remote/remote.component').then(m => m.RemoteComponent) },
      { path: 'git', loadComponent: () => import('./pages/git/git.component').then(m => m.GitComponent) },
      { path: 'jobs', loadComponent: () => import('./pages/jobs/jobs.component').then(m => m.JobsComponent) },
      { path: 'jobs/:id', loadComponent: () => import('./pages/jobs/job-log.component').then(m => m.JobLogComponent) },
    ],
  },
  { path: '**', redirectTo: '' },
];
