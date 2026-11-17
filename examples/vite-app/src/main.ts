import './style.css'

const status = document.querySelector<HTMLParagraphElement>('#status')
const button = document.querySelector<HTMLButtonElement>('#ping')

if (!status || !button) {
  throw new Error('example markup is missing #status or #ping')
}

const statusEl: HTMLParagraphElement = status

const callApi = async (): Promise<void> => {
  statusEl.textContent = 'Calling /api/hello...'
  try {
    const response = await fetch('/api/hello')
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`)
    }
    const message: unknown = await response.json()
    statusEl.textContent = `API says: ${String(message)}`
  } catch (error) {
    statusEl.textContent = `API call failed: ${String(error)}`
  }
}

button.addEventListener('click', () => {
  void callApi()
})

statusEl.textContent = 'Ready. Press the button to call the API.'
