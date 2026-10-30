/**
 * Spec 063 T056: the New binding form picks the project from the workspace's projects instead of
 * asking for its id. A workspace with none sends you to Teams, where each project's id has its own
 * Copy button. Against the fake backend.
 */
import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import { governance, type FakeGovernance } from '../../support/fake-governance'
import { NOW, PROJECT, WORKSPACE } from '../../support/fake-repos'

const OTHER = '01J00000000000000000000PR2'

/** An instance admin granted ws_admin on `ops`, whose workspace holds `projects`. */
function admin(backend: FakeBackend, projects: { id: string; slug: string; name: string }[]): FakeGovernance {
  backend.me = fixtures.me({
    grants: [{ workspace: WORKSPACE, workspace_slug: 'ops', role: 'ws_admin', profiles: [] }],
    verification: fixtures.verification({ verified: true }),
  })
  return governance(backend, (g) => {
    g.workspaces.push({ id: WORKSPACE, slug: 'ops', name: 'Ops', created: NOW, archived: false, projects, system: false })
  })
}

test('the binding is made in the project picked', async ({ page, backend }) => {
  admin(backend, [
    { id: PROJECT, slug: 'infra', name: 'Infra' },
    { id: OTHER, slug: 'apps', name: 'Apps' },
  ])
  await page.goto('/bindings')
  await page.getByRole('button', { name: 'New binding' }).first().click()
  const form = page.getByRole('form', { name: 'New binding' })
  await expect(form.getByLabel('Project id')).toHaveCount(0)
  const project = form.getByLabel('Project')
  await expect(project.locator('option')).toHaveText(['Choose a project…', 'Infra (infra)', 'Apps (apps)'])
  await form.getByLabel('Repository URL').fill('https://git.example.com/ops/apps.git')
  await expect(form.getByRole('button', { name: 'Preview binding' })).toBeDisabled()

  await project.selectOption({ label: 'Apps (apps)' })
  await form.getByRole('button', { name: 'Preview binding' }).click()
  expect((await backend.waitForCmd('PreviewBindingCreate')).data.project).toBe(OTHER)
})

test('one project is picked for you', async ({ page, backend }) => {
  admin(backend, [{ id: PROJECT, slug: 'infra', name: 'Infra' }])
  await page.goto('/bindings')
  await page.getByRole('button', { name: 'New binding' }).first().click()
  const form = page.getByRole('form', { name: 'New binding' })
  await expect(form.getByLabel('Project')).toHaveValue(PROJECT)
  await form.getByLabel('Repository URL').fill('https://git.example.com/ops/infra.git')
  await form.getByRole('button', { name: 'Preview binding' }).click()
  expect((await backend.waitForCmd('PreviewBindingCreate')).data.project).toBe(PROJECT)
})

test('a workspace with no projects sends you to Teams, which shows each id to copy', async ({ page, backend, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  const g = admin(backend, [])
  await page.goto('/bindings')
  await page.getByRole('button', { name: 'New binding' }).first().click()
  const form = page.getByRole('form', { name: 'New binding' })
  await expect(form.locator('[data-no-projects]')).toContainText('This workspace has no projects yet.')
  await expect(form.getByRole('button', { name: 'Preview binding' })).toBeDisabled()
  await form.getByRole('link', { name: 'Create one in Teams' }).click()
  await expect(page).toHaveURL(/\/teams$/)

  // Once the workspace has a project, Teams shows its id with a Copy button.
  g.workspaces[0]?.projects.push({ id: PROJECT, slug: 'infra', name: 'Infra' })
  await page.reload()
  const projects = page.getByRole('list', { name: 'Projects in ops' })
  await expect(projects.locator('[data-copy="project-id"]')).toHaveText(PROJECT)
  const copy = projects.getByRole('button', { name: 'Copy project infra id', exact: true })
  await copy.click()
  await expect(copy).toHaveText('Copied')
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(PROJECT)
})
