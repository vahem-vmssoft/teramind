import { lazy, Suspense } from 'react';
import { createRootRoute, createRoute, Outlet } from '@tanstack/react-router';
import { Shell } from './components/Shell';
import { Activity } from './routes/activity';
import { Skills } from './routes/skills';
import { Members } from './routes/members';
import { Health } from './routes/health';
import { Login } from './routes/login';

const QualityLazy = lazy(() => import('./routes/quality').then(m => ({ default: m.Quality })));

const rootRoute = createRootRoute({ component: () => <Outlet /> });

const loginRoute = createRoute({
  getParentRoute: () => rootRoute, path: '/login', component: Login,
});

const shellRoute = createRoute({
  getParentRoute: () => rootRoute, id: 'shell', component: Shell,
});

const indexRoute  = createRoute({ getParentRoute: () => shellRoute, path: '/',          component: Activity });
const activity    = createRoute({ getParentRoute: () => shellRoute, path: '/activity',  component: Activity });
const skills      = createRoute({ getParentRoute: () => shellRoute, path: '/skills',    component: Skills });
const members     = createRoute({ getParentRoute: () => shellRoute, path: '/members',   component: Members });
const quality     = createRoute({
  getParentRoute: () => shellRoute, path: '/quality',
  component: () => (
    <Suspense fallback={<div className="text-sm text-neutral-500">Loading…</div>}>
      <QualityLazy />
    </Suspense>
  ),
});
const health      = createRoute({ getParentRoute: () => shellRoute, path: '/health',    component: Health });

export const routeTree = rootRoute.addChildren([
  loginRoute,
  shellRoute.addChildren([indexRoute, activity, skills, members, quality, health]),
]);
