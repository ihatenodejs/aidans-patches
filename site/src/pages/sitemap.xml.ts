import type { APIRoute } from 'astro';
import { getApps } from '../data/repo';

export const prerender = true;

export const GET: APIRoute = ({ site }) => {
  const origin = site || new URL('https://patch.p0ntus.com');
  const routes = [
    { path: '/', changefreq: 'daily', priority: '1.0' },
    { path: '/apps', changefreq: 'weekly', priority: '0.9' },
    ...getApps().map((app) => ({
      path: `/apps/${app.id}`,
      changefreq: 'weekly',
      priority: '0.8',
    })),
    { path: '/status', changefreq: 'hourly', priority: '0.8' },
  ];
  const urls = routes
    .map(
      ({ path, changefreq, priority }) => `  <url>
    <loc>${new URL(path, origin).href}</loc>
    <changefreq>${changefreq}</changefreq>
    <priority>${priority}</priority>
  </url>`,
    )
    .join('\n');

  return new Response(
    `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`,
    { headers: { 'Content-Type': 'application/xml; charset=utf-8' } },
  );
};
