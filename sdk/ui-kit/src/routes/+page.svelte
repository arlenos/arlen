<script lang="ts">
  import { Button } from "$lib/components/ui/button/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "$lib/components/ui/card/index.js";
  import { Separator } from "$lib/components/ui/separator/index.js";
</script>

<div class="p-8 max-w-2xl mx-auto space-y-8">

  <div>
    <h1 class="text-2xl font-semibold tracking-tight">Lunaris ui-kit</h1>
    <p class="text-muted-foreground mt-1">
      Shared component library for Lunaris first-party apps.
      Built with shadcn-svelte, Tailwind v4, and Lunaris Surface Tokens.
    </p>
  </div>

  <Separator />

  <!-- Buttons -->
  <div class="space-y-3">
    <h2 class="text-sm font-medium text-muted-foreground uppercase tracking-wider">Buttons</h2>
    <div class="flex flex-wrap gap-3">
      <Button>Default</Button>
      <Button variant="secondary">Secondary</Button>
      <Button variant="outline">Outline</Button>
      <Button variant="ghost">Ghost</Button>
      <Button variant="destructive">Destructive</Button>
    </div>
  </div>

  <Separator />

  <!-- Badges -->
  <div class="space-y-3">
    <h2 class="text-sm font-medium text-muted-foreground uppercase tracking-wider">Badges</h2>
    <div class="flex flex-wrap gap-3">
      <Badge>Default</Badge>
      <Badge variant="secondary">Secondary</Badge>
      <Badge variant="outline">Outline</Badge>
      <Badge variant="destructive">Destructive</Badge>
    </div>
  </div>

  <Separator />

  <!-- Cards -->
  <div class="space-y-3">
    <h2 class="text-sm font-medium text-muted-foreground uppercase tracking-wider">Cards</h2>
    <div class="grid grid-cols-2 gap-4">
      <Card>
        <CardHeader>
          <CardTitle>Surface Tokens</CardTitle>
          <CardDescription>
            All colors come from theme.toml via CSS custom properties.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div class="grid grid-cols-3 gap-2">
            <div class="h-8 rounded" style="background: var(--color-bg-shell)"></div>
            <div class="h-8 rounded border" style="background: var(--color-bg-app)"></div>
            <div class="h-8 rounded" style="background: var(--color-bg-card)"></div>
            <div class="h-8 rounded" style="background: var(--color-accent)"></div>
            <div class="h-8 rounded" style="background: var(--color-border)"></div>
            <div class="h-8 rounded" style="background: var(--color-bg-input)"></div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Panda Theme</CardTitle>
          <CardDescription>
            Dark shell, light apps. The default Lunaris theme.
          </CardDescription>
        </CardHeader>
        <CardContent class="space-y-2">
          <div class="shell-surface rounded p-3 text-sm">
            Shell surface
          </div>
          <div class="rounded border p-3 text-sm">
            App surface
          </div>
        </CardContent>
      </Card>
    </div>
  </div>

</div>
