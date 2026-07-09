// ============================================================================
// ProvidersStep — render and interaction tests
// ============================================================================

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import type { Transport } from '@/services/transport';

const createProvider = vi.fn<Transport['createProvider']>();
const testProviderById = vi.fn<Transport['testProviderById']>();
const deleteProvider = vi.fn<Transport['deleteProvider']>();
const listProviders = vi.fn<Transport['listProviders']>();

vi.mock('@/services/transport', () => ({
  getTransport: async () => ({ createProvider, testProviderById, deleteProvider, listProviders }),
}));

import { ProvidersStep } from './ProvidersStep';

function makeProvider(id: string, name: string, baseUrl: string, verified = true) {
  return { id, name, baseUrl, models: ['m1', 'm2'], defaultModel: 'm1', enabled: true, isDefault: false, verified };
}

const defaultProps = {
  providers: [],
  defaultProviderId: '',
  onProvidersChanged: vi.fn(),
};

beforeEach(() => {
  createProvider.mockReset();
  testProviderById.mockReset();
  deleteProvider.mockReset();
  listProviders.mockReset();
  defaultProps.onProvidersChanged = vi.fn();
});

describe('ProvidersStep', () => {
  it('renders the preset grid with provider cards', () => {
    render(<ProvidersStep {...defaultProps} />);
    // OpenAI, Anthropic, and Ollama Cloud are known presets
    expect(screen.getByText('OpenAI')).toBeInTheDocument();
    expect(screen.getByText('Anthropic')).toBeInTheDocument();
  });

  it('shows already-added providers in the list', () => {
    render(
      <ProvidersStep
        {...defaultProps}
        providers={[makeProvider('p1', 'OpenAI', 'https://api.openai.com/v1')]}
        defaultProviderId="p1"
      />
    );
    // OpenAI appears in the added list AND in the preset grid
    expect(screen.getAllByText('OpenAI').length).toBeGreaterThan(0);
    // default badge
    expect(screen.getByText('default')).toBeInTheDocument();
  });

  it('shows verified badge for verified providers', () => {
    render(
      <ProvidersStep
        {...defaultProps}
        providers={[makeProvider('p1', 'OpenAI', 'https://api.openai.com/v1', true)]}
        defaultProviderId="p1"
      />
    );
    expect(screen.getByText('verified')).toBeInTheDocument();
  });

  it('shows "set default" button for non-default providers', () => {
    render(
      <ProvidersStep
        {...defaultProps}
        providers={[
          makeProvider('p1', 'OpenAI', 'https://api.openai.com/v1'),
          makeProvider('p2', 'Anthropic', 'https://api.anthropic.com/v1'),
        ]}
        defaultProviderId="p1"
      />
    );
    expect(screen.getAllByText('set default').length).toBeGreaterThan(0);
  });

  it('calls onProvidersChanged with new defaultId when "set default" is clicked', () => {
    const onProvidersChanged = vi.fn();
    const providers = [
      makeProvider('p1', 'OpenAI', 'https://api.openai.com/v1'),
      makeProvider('p2', 'Anthropic', 'https://api.anthropic.com/v1'),
    ];
    render(
      <ProvidersStep
        providers={providers}
        defaultProviderId="p1"
        onProvidersChanged={onProvidersChanged}
      />
    );
    fireEvent.click(screen.getByText('set default'));
    expect(onProvidersChanged).toHaveBeenCalledWith(providers, 'p2');
  });

  it('removes a provider when "remove" is clicked', async () => {
    const onProvidersChanged = vi.fn();
    deleteProvider.mockResolvedValue({ success: true });
    listProviders.mockResolvedValue({ success: true, data: [] });

    render(
      <ProvidersStep
        providers={[makeProvider('p1', 'OpenAI', 'https://api.openai.com/v1')]}
        defaultProviderId="p1"
        onProvidersChanged={onProvidersChanged}
      />
    );
    fireEvent.click(screen.getByText('remove'));
    await waitFor(() => {
      expect(deleteProvider).toHaveBeenCalledWith('p1');
      expect(onProvidersChanged).toHaveBeenCalledWith([], '');
    });
  });

  it('expands inline form when a provider card with API key is clicked', () => {
    render(<ProvidersStep {...defaultProps} />);
    fireEvent.click(screen.getByText('OpenAI'));
    expect(screen.getByText('Add OpenAI')).toBeInTheDocument();
    expect(screen.getByPlaceholderText('sk-...')).toBeInTheDocument();
  });

  it('disables Test & Add button when API key is empty', () => {
    render(<ProvidersStep {...defaultProps} />);
    fireEvent.click(screen.getByText('OpenAI'));
    expect(screen.getByText('Test & Add')).toBeDisabled();
  });

  it('enables Test & Add button when API key is filled in', () => {
    render(<ProvidersStep {...defaultProps} />);
    fireEvent.click(screen.getByText('OpenAI'));
    fireEvent.change(screen.getByPlaceholderText('sk-...'), { target: { value: 'sk-abc123' } });
    expect(screen.getByText('Test & Add')).not.toBeDisabled();
  });

  it('shows error when createProvider fails', async () => {
    createProvider.mockResolvedValue({ success: false, error: 'Invalid API key' });
    render(<ProvidersStep {...defaultProps} />);
    fireEvent.click(screen.getByText('OpenAI'));
    fireEvent.change(screen.getByPlaceholderText('sk-...'), { target: { value: 'sk-bad' } });
    fireEvent.click(screen.getByText('Test & Add'));
    await waitFor(() => {
      expect(screen.getByText('Invalid API key')).toBeInTheDocument();
    });
  });

  it('shows error when test fails and deletes the provider', async () => {
    createProvider.mockResolvedValue({ success: true, data: { id: 'p-new', name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', models: [], defaultModel: '', enabled: true, isDefault: false } });
    testProviderById.mockResolvedValue({ success: true, data: { success: false, message: 'Auth failed' } });
    deleteProvider.mockResolvedValue({ success: true });
    render(<ProvidersStep {...defaultProps} />);
    fireEvent.click(screen.getByText('OpenAI'));
    fireEvent.change(screen.getByPlaceholderText('sk-...'), { target: { value: 'sk-bad' } });
    fireEvent.click(screen.getByText('Test & Add'));
    await waitFor(() => {
      expect(deleteProvider).toHaveBeenCalledWith('p-new');
      expect(screen.getByText('Auth failed')).toBeInTheDocument();
    });
  });

  it('adds provider successfully and calls onProvidersChanged', async () => {
    const onProvidersChanged = vi.fn();
    const newProvider = makeProvider('p-new', 'OpenAI', 'https://api.openai.com/v1');
    createProvider.mockResolvedValue({
      success: true,
      data: { id: 'p-new', name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', models: [], defaultModel: '', enabled: true, isDefault: false },
    });
    testProviderById.mockResolvedValue({ success: true, data: { success: true, message: 'OK' } });
    listProviders.mockResolvedValue({ success: true, data: [newProvider] });

    render(<ProvidersStep providers={[]} defaultProviderId="" onProvidersChanged={onProvidersChanged} />);
    fireEvent.click(screen.getByText('OpenAI'));
    fireEvent.change(screen.getByPlaceholderText('sk-...'), { target: { value: 'sk-good' } });
    fireEvent.click(screen.getByText('Test & Add'));
    await waitFor(() => {
      expect(onProvidersChanged).toHaveBeenCalledWith([newProvider], 'p-new');
    });
  });

  it('adds noApiKey provider directly (Ollama Cloud)', async () => {
    const onProvidersChanged = vi.fn();
    const ollamaProvider = makeProvider('p-oll', 'Ollama Cloud', 'http://localhost:11434/v1');
    createProvider.mockResolvedValue({
      success: true,
      data: { id: 'p-oll', name: 'Ollama Cloud', baseUrl: 'http://localhost:11434/v1', models: [], defaultModel: '', enabled: true, isDefault: false },
    });
    testProviderById.mockResolvedValue({ success: true, data: { success: true, message: 'OK' } });
    listProviders.mockResolvedValue({ success: true, data: [ollamaProvider] });

    render(<ProvidersStep providers={[]} defaultProviderId="" onProvidersChanged={onProvidersChanged} />);
    // Ollama Cloud has noApiKey — clicking it should directly test+add without showing form
    fireEvent.click(screen.getAllByText('Ollama Cloud')[0]);
    await waitFor(() => {
      expect(createProvider).toHaveBeenCalled();
    });
  });

  it('does not mark Ollama Local as added just because Ollama Cloud uses the same base URL', () => {
    render(
      <ProvidersStep
        providers={[makeProvider('p-oll-cloud', 'Ollama Cloud', 'http://localhost:11434/v1')]}
        defaultProviderId="p-oll-cloud"
        onProvidersChanged={vi.fn()}
      />
    );

    const localCard = screen.getByText('Ollama Local').closest('[role="button"]')!;
    expect(localCard).not.toHaveClass('provider-add-card--added');
    expect(localCard).toHaveTextContent('no key needed');
  });

  it('shows "added" hint for already-added presets', () => {
    render(
      <ProvidersStep
        providers={[makeProvider('p1', 'OpenAI', 'https://api.openai.com/v1')]}
        defaultProviderId="p1"
        onProvidersChanged={vi.fn()}
      />
    );
    // The card for OpenAI in the preset grid should show "added"
    expect(screen.getByText('added')).toBeInTheDocument();
  });
});
